use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pdfword_core::{ConvertOptions, Fidelity, TargetFormat, DEFAULT_EXACT_LOOK_DPI};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

const INDEX_HTML: &str = include_str!("../static/index.html");
const STYLE_CSS: &str = include_str!("../static/style.css");
const APP_JS: &str = include_str!("../static/app.js");
const FAVICON_BYTES: &[u8] = include_bytes!("../static/favicon.png");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum JobStatus {
    Queued,
    Processing,
    Done,
    Failed,
}

#[derive(Debug, Clone)]
struct Job {
    id: String,
    status: JobStatus,
    input_name: String,
    output_name: String,
    output_path: Option<PathBuf>,
    error: Option<String>,
    created: Instant,
}

#[derive(Serialize)]
struct JobView {
    id: String,
    status: JobStatus,
    input_name: String,
    output_name: String,
    error: Option<String>,
}

impl Job {
    fn view(&self) -> JobView {
        JobView {
            id: self.id.clone(),
            status: self.status,
            input_name: self.input_name.clone(),
            output_name: self.output_name.clone(),
            error: self.error.clone(),
        }
    }
}

#[derive(Serialize)]
struct CreateJobResponse {
    id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct UserStats {
    base_count: usize,
    unique_users: HashSet<String>,
    total_conversions: usize,
}

impl UserStats {
    fn total_users(&self) -> usize {
        self.base_count + self.unique_users.len()
    }
}

#[derive(Serialize)]
struct StatsResponse {
    users: usize,
    conversions: usize,
}

#[derive(Deserialize)]
struct PingRequest {
    uid: Option<String>,
}

async fn save_stats(stats: &UserStats, path: &FsPath) {
    if let Ok(json) = serde_json::to_string_pretty(stats) {
        let _ = tokio::fs::write(path, json).await;
    }
}

#[derive(Clone)]
struct AppState {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    jobs_dir: Arc<PathBuf>,
    semaphore: Arc<Semaphore>,
    max_upload: usize,
    job_ttl: Duration,
    convert_timeout: Duration,
    soffice: Option<PathBuf>,
    pdfium: Option<PathBuf>,
    exact_look_dpi: f32,
    stats: Arc<Mutex<UserStats>>,
    stats_file: Arc<PathBuf>,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn payload_too_large() -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "upload is too large")
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    fn from_io(error: std::io::Error) -> Self {
        Self::internal(error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn stylesheet() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        APP_JS,
    )
}

async fn static_asset(Path(file): Path<String>) -> Result<Response, StatusCode> {
    if file.contains("..") || file.contains('/') || file.contains('\\') {
        return Err(StatusCode::BAD_REQUEST);
    }

    let static_dir = FsPath::new("crates/web/static");
    let file_path = static_dir.join(&file);
    if file_path.is_file() {
        if let Ok(bytes) = tokio::fs::read(&file_path).await {
            let mime = if file.ends_with(".png") {
                "image/png"
            } else if file.ends_with(".ico") {
                "image/x-icon"
            } else if file.ends_with(".jpg") || file.ends_with(".jpeg") {
                "image/jpeg"
            } else if file.ends_with(".svg") {
                "image/svg+xml"
            } else if file.ends_with(".webp") {
                "image/webp"
            } else if file.ends_with(".css") {
                "text/css; charset=utf-8"
            } else if file.ends_with(".js") {
                "text/javascript; charset=utf-8"
            } else {
                "application/octet-stream"
            };
            return Ok(([(header::CONTENT_TYPE, mime)], bytes).into_response());
        }
    }

    Err(StatusCode::NOT_FOUND)
}

async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=86400")),
        ],
        FAVICON_BYTES,
    )
}

async fn health() -> &'static str {
    "ok"
}

fn extension(name: &str) -> String {
    FsPath::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

fn safe_stem(name: &str) -> String {
    let stem = FsPath::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("document");

    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();

    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "document".to_string()
    } else {
        cleaned
    }
}

async fn create_job(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<CreateJobResponse>, ApiError> {
    let mut upload: Option<(String, Vec<u8>)> = None;
    let mut target: Option<String> = None;
    let mut fidelity: Option<String> = None;
    let mut uid: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::bad_request(format!("invalid multipart body: {error}")))?
    {
        let field_name = field.name().unwrap_or_default().to_string();

        match field_name.as_str() {
            "file" => {
                let name = field.file_name().unwrap_or("upload").to_string();
                let bytes = field.bytes().await.map_err(|error| {
                    ApiError::bad_request(format!("could not read the upload: {error}"))
                })?;

                if bytes.len() > state.max_upload {
                    return Err(ApiError::payload_too_large());
                }

                upload = Some((name, bytes.to_vec()));
            }
            "target" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request(format!("could not read the target field: {error}"))
                })?;
                target = Some(value);
            }
            "fidelity" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request(format!("could not read the fidelity field: {error}"))
                })?;
                fidelity = Some(value);
            }
            "uid" => {
                let value = field.text().await.map_err(|error| {
                    ApiError::bad_request(format!("could not read the uid field: {error}"))
                })?;
                uid = Some(value);
            }
            _ => {
                let _ = field.bytes().await;
            }
        }
    }

    let (input_name, bytes) =
        upload.ok_or_else(|| ApiError::bad_request("missing \"file\" field"))?;

    let target = match target.as_deref() {
        Some("docx") => TargetFormat::Docx,
        Some("pdf") => TargetFormat::Pdf,
        _ => return Err(ApiError::bad_request("target must be \"docx\" or \"pdf\"")),
    };

    let input_extension = extension(&input_name);
    let source_ok = match target {
        TargetFormat::Docx => matches!(input_extension.as_str(), "pdf" | "wps" | "wpt"),
        TargetFormat::Pdf => matches!(input_extension.as_str(), "docx" | "doc" | "wps" | "wpt"),
    };
    if !source_ok {
        let expected = match target {
            TargetFormat::Docx => "a PDF (.pdf) or WPS document (.wps or .wpt)",
            TargetFormat::Pdf => "a Word (.docx or .doc) or WPS document (.wps or .wpt)",
        };
        return Err(ApiError::bad_request(format!(
            "this mode expects {expected}"
        )));
    }

    let fidelity = if target == TargetFormat::Docx && input_extension == "pdf" {
        match fidelity.as_deref() {
            None | Some("editable") => Fidelity::Editable,
            Some("exact") => Fidelity::ExactLook,
            Some(_) => {
                return Err(ApiError::bad_request(
                    "fidelity must be \"editable\" or \"exact\"",
                ));
            }
        }
    } else {
        Fidelity::Editable
    };

    let id = Uuid::new_v4().simple().to_string();
    let job_dir = state.jobs_dir.join(&id);
    tokio::fs::create_dir_all(&job_dir)
        .await
        .map_err(ApiError::from_io)?;

    let input_path = job_dir.join(format!("input.{input_extension}"));
    let output_path = job_dir.join(format!("output.{}", target.extension()));
    tokio::fs::write(&input_path, &bytes)
        .await
        .map_err(ApiError::from_io)?;

    let output_name = format!("{}.{}", safe_stem(&input_name), target.extension());
    let job = Job {
        id: id.clone(),
        status: JobStatus::Queued,
        input_name,
        output_name,
        output_path: Some(output_path.clone()),
        error: None,
        created: Instant::now(),
    };

    state.jobs.lock().await.insert(id.clone(), job);

    let job_state = state.clone();
    let job_id = id.clone();

    tokio::spawn(async move {
        let permit = match job_state.semaphore.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };

        {
            let mut jobs = job_state.jobs.lock().await;
            if let Some(job) = jobs.get_mut(&job_id) {
                job.status = JobStatus::Processing;
            }
        }

        let soffice = job_state.soffice.clone();
        let pdfium = job_state.pdfium.clone();
        let timeout = job_state.convert_timeout;
        let exact_look_dpi = job_state.exact_look_dpi;
        let input = input_path.clone();
        let output = output_path.clone();

        let result = tokio::task::spawn_blocking(move || {
            let options = ConvertOptions {
                soffice_path: soffice,
                pdfium_dir: pdfium,
                timeout: Some(timeout),
                fidelity,
                exact_look_dpi,
            };
            pdfword_core::convert(&input, &output, target, &options)
        })
        .await;

        let mut jobs = job_state.jobs.lock().await;
        if let Some(job) = jobs.get_mut(&job_id) {
            match result {
                Ok(Ok(())) => {
                    job.status = JobStatus::Done;
                    job.error = None;
                }
                Ok(Err(error)) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(error.to_string());
                }
                Err(error) => {
                    job.status = JobStatus::Failed;
                    job.error = Some(format!("conversion task failed: {error}"));
                }
            }

            if job.status == JobStatus::Failed {
                if let Some(directory) = job.output_path.as_deref().and_then(FsPath::parent) {
                    let _ = std::fs::remove_dir_all(directory);
                }
            }
        }

        drop(jobs);
        drop(permit);
    });

    {
        let mut stats = state.stats.lock().await;
        stats.total_conversions += 1;
        if let Some(user_id) = uid {
            let trimmed = user_id.trim();
            if !trimmed.is_empty() && trimmed.len() <= 128 {
                stats.unique_users.insert(trimmed.to_string());
            }
        }
        save_stats(&stats, &state.stats_file).await;
    }

    Ok(Json(CreateJobResponse { id }))
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let stats = state.stats.lock().await;
    Json(StatsResponse {
        users: stats.total_users(),
        conversions: stats.total_conversions,
    })
}

async fn ping_stats(
    State(state): State<AppState>,
    Json(payload): Json<PingRequest>,
) -> Json<StatsResponse> {
    let mut stats = state.stats.lock().await;
    let mut changed = false;

    if let Some(uid) = payload.uid {
        let trimmed = uid.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= 128
            && stats.unique_users.insert(trimmed.to_string())
        {
            changed = true;
        }
    }

    if changed {
        save_stats(&stats, &state.stats_file).await;
    }

    Json(StatsResponse {
        users: stats.total_users(),
        conversions: stats.total_conversions,
    })
}

async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<JobView>, ApiError> {
    let jobs = state.jobs.lock().await;
    let job = jobs
        .get(&id)
        .ok_or_else(|| ApiError::not_found("job not found"))?;

    Ok(Json(job.view()))
}

async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (status, path, name) = {
        let jobs = state.jobs.lock().await;
        let job = jobs
            .get(&id)
            .ok_or_else(|| ApiError::not_found("job not found"))?;

        (job.status, job.output_path.clone(), job.output_name.clone())
    };

    if status != JobStatus::Done {
        return Err(ApiError::bad_request("job is not finished yet"));
    }

    let path = path.ok_or_else(|| ApiError::internal("job has no output file"))?;
    let bytes = tokio::fs::read(&path).await.map_err(ApiError::from_io)?;

    let mime = if name.ends_with(".pdf") {
        "application/pdf"
    } else {
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    };

    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));

    if let Ok(value) = HeaderValue::from_str(&format!("attachment; filename=\"{name}\"")) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }

    Ok(response)
}

fn spawn_cleanup(state: AppState) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(300)).await;

            let expired: Vec<String> = {
                let jobs = state.jobs.lock().await;
                let now = Instant::now();

                jobs.iter()
                    .filter(|(_, job)| now.duration_since(job.created) > state.job_ttl)
                    .map(|(id, _)| id.clone())
                    .collect()
            };

            for id in expired {
                let directory = {
                    let mut jobs = state.jobs.lock().await;
                    jobs.remove(&id)
                        .and_then(|job| job.output_path)
                        .and_then(|path| path.parent().map(FsPath::to_path_buf))
                };

                if let Some(directory) = directory {
                    let _ = tokio::fs::remove_dir_all(directory).await;
                }
            }
        }
    });
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(default)
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let work_root = env_path("WORK_DIR").unwrap_or_else(|| std::env::temp_dir().join("pdfword"));
    let jobs_dir = work_root.join("jobs");
    std::fs::create_dir_all(&jobs_dir).expect("could not create the work directory");

    let max_upload_mb = env_usize("MAX_UPLOAD_MB", 25);

    let stats_file = work_root.join("stats.json");
    let stats: UserStats = if stats_file.is_file() {
        match std::fs::read_to_string(&stats_file) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => UserStats::default(),
        }
    } else {
        UserStats::default()
    };

    let state = AppState {
        jobs: Arc::new(Mutex::new(HashMap::new())),
        jobs_dir: Arc::new(jobs_dir),
        semaphore: Arc::new(Semaphore::new(env_usize("MAX_CONCURRENT", 2))),
        max_upload: max_upload_mb * 1024 * 1024,
        job_ttl: Duration::from_secs(env_usize("JOB_TTL_SECS", 3600) as u64),
        convert_timeout: Duration::from_secs(env_usize("CONVERT_TIMEOUT_SECS", 120) as u64),
        soffice: env_path("SOFFICE_PATH"),
        pdfium: env_path("PDFIUM_PATH"),
        exact_look_dpi: env_f32("EXACT_LOOK_DPI", DEFAULT_EXACT_LOOK_DPI),
        stats: Arc::new(Mutex::new(stats)),
        stats_file: Arc::new(stats_file),
    };

    spawn_cleanup(state.clone());

    let app = Router::new()
        .route("/", get(index))
        .route("/favicon.ico", get(favicon))
        .route("/favicon.png", get(favicon))
        .route("/static/favicon.png", get(favicon))
        .route("/static/favicon.ico", get(favicon))
        .route("/static/style.css", get(stylesheet))
        .route("/static/app.js", get(script))
        .route("/static/{file}", get(static_asset))
        .route("/healthz", get(health))
        .route("/api/stats", get(get_stats))
        .route("/api/stats/ping", post(ping_stats))
        .route("/api/jobs", post(create_job))
        .route("/api/jobs/{id}", get(get_job))
        .route("/api/jobs/{id}/download", get(download))
        .layer(DefaultBodyLimit::max(state.max_upload + 1024 * 1024))
        .with_state(state);

    let bind = std::env::var("BIND_ADDR")
        .ok()
        .or_else(|| std::env::var("PORT").ok().map(|p| format!("0.0.0.0:{p}")))
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());
    let address: SocketAddr = bind.parse().expect("invalid BIND_ADDR");
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("could not bind the listener");

    tracing::info!("pdfword listening on http://{address}");
    axum::serve(listener, app).await.expect("server error");
}
