use axum::{
    extract::{Query, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    process::Command,
    fs,
    path::Path as StdPath,
    time::{SystemTime, UNIX_EPOCH},
    sync::{Arc, Mutex},
};
use std::net::SocketAddr;
use tokio::time::{sleep, Duration};

#[derive(Deserialize)]
struct SaveParams {
    url: String,
}

#[derive(Clone)]
struct CacheEntry {
    file_path: String,
    timestamp: u64,
}

#[derive(Clone)]
enum TaskStatus {
    Queued(usize), // 队列中的位置
    Processing,
    Completed(String), // 文件路径
    Failed(String), // 错误信息
}

#[derive(Serialize)]
struct TaskResponse {
    task_id: String,
    status: String,
    message: String,
    queue_position: Option<usize>,
    file_url: Option<String>,
}

#[derive(Serialize)]
struct QueueStatusResponse {
    task_id: String,
    status: String,
    message: String,
    queue_position: Option<usize>,
}

type Cache = Arc<Mutex<HashMap<String, CacheEntry>>>;
type TaskQueue = Arc<Mutex<VecDeque<String>>>;
type TaskStatuses = Arc<Mutex<HashMap<String, TaskStatus>>>;

struct AppState {
    cache: Cache,
    task_queue: TaskQueue,
    task_statuses: TaskStatuses,
    processing: Arc<Mutex<bool>>,
}

fn generate_task_id() -> String {
    use std::time::SystemTime;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    format!("task_{}", timestamp)
}

fn url_to_cache_key(url: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    format!("cache_{}", hasher.finish())
}

fn get_current_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

async fn save_page(
    Query(params): Query<SaveParams>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    if params.url.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(TaskResponse {
            task_id: "".to_string(),
            status: "error".to_string(),
            message: "Missing url parameter".to_string(),
            queue_position: None,
            file_url: None,
        })).into_response();
    }

    let cache_key = url_to_cache_key(&params.url);
    let current_time = get_current_timestamp();

    // 检查缓存
    {
        let cache = state.cache.lock().unwrap();
        if let Some(entry) = cache.get(&cache_key) {
            // 检查是否在24小时内
            if current_time - entry.timestamp < 24 * 3600 && StdPath::new(&entry.file_path).exists() {
                println!("Cache hit for URL: {}", params.url);
                return Json(TaskResponse {
                    task_id: cache_key.clone(),
                    status: "completed".to_string(),
                    message: "Retrieved from cache".to_string(),
                    queue_position: None,
                    file_url: Some(format!("/files/{}", cache_key)),
                }).into_response();
            }
        }
    }

    // 生成任务ID并添加到队列
    let task_id = generate_task_id();

    {
        let mut queue = state.task_queue.lock().unwrap();
        let mut statuses = state.task_statuses.lock().unwrap();

        queue.push_back(task_id.clone());
        let queue_position = queue.len();
        statuses.insert(task_id.clone(), TaskStatus::Queued(queue_position));

        // 存储URL信息（用于处理时获取）
        statuses.insert(format!("url_{}", task_id), TaskStatus::Completed(params.url.clone()));
    }

    // 启动处理器（如果还没在运行）
    tokio::spawn(process_queue(state.clone()));

    let queue_position = {
        let queue = state.task_queue.lock().unwrap();
        queue.len()
    };

    Json(TaskResponse {
        task_id,
        status: "queued".to_string(),
        message: format!("Task queued successfully"),
        queue_position: Some(queue_position),
        file_url: None,
    }).into_response()
}

async fn get_task_status(
    Path(task_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let statuses = state.task_statuses.lock().unwrap();

    match statuses.get(&task_id) {
        Some(TaskStatus::Queued(position)) => Json(QueueStatusResponse {
            task_id,
            status: "queued".to_string(),
            message: format!("Waiting in queue"),
            queue_position: Some(*position),
        }).into_response(),
        Some(TaskStatus::Processing) => Json(QueueStatusResponse {
            task_id,
            status: "processing".to_string(),
            message: "Currently processing your request".to_string(),
            queue_position: None,
        }).into_response(),
        Some(TaskStatus::Completed(file_path)) => {
            Json(QueueStatusResponse {
                task_id,
                status: "completed".to_string(),
                message: "Task completed successfully".to_string(),
                queue_position: None,
            }).into_response()
        },
        Some(TaskStatus::Failed(error)) => Json(QueueStatusResponse {
            task_id,
            status: "failed".to_string(),
            message: error.clone(),
            queue_position: None,
        }).into_response(),
        None => (StatusCode::NOT_FOUND, Json(QueueStatusResponse {
            task_id,
            status: "not_found".to_string(),
            message: "Task not found".to_string(),
            queue_position: None,
        })).into_response(),
    }
}

async fn get_file(
    Path(file_id): Path<String>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    // 首先检查是否是缓存键
    let file_path = {
        let cache = state.cache.lock().unwrap();
        if let Some(entry) = cache.get(&file_id) {
            entry.file_path.clone()
        } else {
            format!("/tmp/{}.html", file_id)
        }
    };

    if !StdPath::new(&file_path).exists() {
        return (StatusCode::NOT_FOUND, "File not found or still processing").into_response();
    }

    match fs::read_to_string(&file_path) {
        Ok(content) => Html(content).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read file").into_response(),
    }
}

async fn process_queue(state: Arc<AppState>) {
    // 防止多个处理器同时运行
    {
        let mut processing = state.processing.lock().unwrap();
        if *processing {
            return;
        }
        *processing = true;
    }

    loop {
        let task_id = {
            let mut queue = state.task_queue.lock().unwrap();
            queue.pop_front()
        };

        match task_id {
            Some(id) => {
                // 更新状态为处理中
                {
                    let mut statuses = state.task_statuses.lock().unwrap();
                    statuses.insert(id.clone(), TaskStatus::Processing);
                }

                // 获取原始URL
                let url = {
                    let statuses = state.task_statuses.lock().unwrap();
                    match statuses.get(&format!("url_{}", id)) {
                        Some(TaskStatus::Completed(url)) => url.clone(),
                        _ => {
                            let mut statuses = state.task_statuses.lock().unwrap();
                            statuses.insert(id.clone(), TaskStatus::Failed("URL not found".to_string()));
                            continue;
                        }
                    }
                };

                // 处理任务
                process_single_task(&id, &url, state.clone()).await;
            }
            None => {
                // 队列为空，等待一会儿再检查
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn process_single_task(task_id: &str, url: &str, state: Arc<AppState>) {
    let output_file = format!("/tmp/{}.html", task_id);
    let cache_key = url_to_cache_key(url);

    println!("Processing task {}: {}", task_id, url);

    let output = Command::new("single-file")
        .arg(url)
        .arg(&output_file)
        .output();

    match output {
        Ok(result) => {
            if result.status.success() && StdPath::new(&output_file).exists() {
                // 成功完成
                {
                    let mut statuses = state.task_statuses.lock().unwrap();
                    statuses.insert(task_id.to_string(), TaskStatus::Completed(output_file.clone()));
                }

                // 添加到缓存
                {
                    let mut cache = state.cache.lock().unwrap();
                    cache.insert(cache_key, CacheEntry {
                        file_path: output_file,
                        timestamp: get_current_timestamp(),
                    });
                }

                println!("Task {} completed successfully", task_id);
            } else {
                // 处理失败
                let error_msg = format!("single-file failed: {}", String::from_utf8_lossy(&result.stderr));
                let mut statuses = state.task_statuses.lock().unwrap();
                statuses.insert(task_id.to_string(), TaskStatus::Failed(error_msg));
                println!("Task {} failed", task_id);
            }
        }
        Err(e) => {
            let mut statuses = state.task_statuses.lock().unwrap();
            statuses.insert(task_id.to_string(), TaskStatus::Failed(format!("Command execution failed: {}", e)));
            println!("Task {} failed to execute", task_id);
        }
    }
}

async fn cleanup_old_files(state: Arc<AppState>) {
    loop {
        sleep(Duration::from_secs(3600)).await; // 每小时清理一次

        let current_time = get_current_timestamp();
        let mut to_remove = Vec::new();

        {
            let cache = state.cache.lock().unwrap();
            for (key, entry) in cache.iter() {
                if current_time - entry.timestamp > 24 * 3600 {
                    to_remove.push((key.clone(), entry.file_path.clone()));
                }
            }
        }

        for (key, file_path) in to_remove {
            // 删除文件
            let _ = fs::remove_file(&file_path);

            // 从缓存中移除
            let mut cache = state.cache.lock().unwrap();
            cache.remove(&key);

            println!("Cleaned up old file: {}", file_path);
        }
    }
}

#[tokio::main]
async fn main() {
    let state = Arc::new(AppState {
        cache: Arc::new(Mutex::new(HashMap::new())),
        task_queue: Arc::new(Mutex::new(VecDeque::new())),
        task_statuses: Arc::new(Mutex::new(HashMap::new())),
        processing: Arc::new(Mutex::new(false)),
    });

    // 启动清理任务
    tokio::spawn(cleanup_old_files(state.clone()));

    let app = Router::new()
        .route("/save", get(save_page))
        .route("/status/:task_id", get(get_task_status))
        .route("/files/:file_id", get(get_file))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    println!("Listening on {}", addr);
    println!("API Endpoints:");
    println!("  POST /save?url=<URL>     - Submit a save task");
    println!("  GET  /status/<task_id>   - Check task status");
    println!("  GET  /files/<file_id>    - Download saved file");

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

