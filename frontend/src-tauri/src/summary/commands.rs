use crate::database::repositories::{
    meeting::MeetingsRepository, summary::SummaryProcessesRepository,
    transcript_chunk::TranscriptChunksRepository,
};
use crate::state::AppState;
use crate::summary::service::SummaryService;
use log::{error as log_error, info as log_info, warn as log_warn};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};
use futures::FutureExt;

#[derive(Debug, Serialize, Deserialize)]
pub struct SummaryResponse {
    pub status: String,
    #[serde(rename = "meetingName")]
    pub meeting_name: Option<String>,
    pub meeting_id: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessTranscriptResponse {
    pub message: String,
    pub process_id: String,
}

/// Saves a meeting summary (Native SQLx implementation)
///
/// Expected format: { "markdown": "...", "summary_json": [...BlockNote blocks...] }
#[tauri::command]
pub async fn api_save_meeting_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    summary: serde_json::Value,
    _auth_token: Option<String>,
) -> Result<serde_json::Value, String> {
    log_info!(
        "api_save_meeting_summary (native) called for meeting_id: {}",
        meeting_id
    );
    let pool = state.db_manager.pool();

    match SummaryProcessesRepository::update_meeting_summary(pool, &meeting_id, &summary).await {
        Ok(true) => {
            log_info!("Summary saved successfully for meeting_id: {}", meeting_id);
            Ok(serde_json::json!({
                "message": "Meeting summary saved successfully"
            }))
        }
        Ok(false) => {
            log_warn!(
                "Meeting not found or invalid JSON for meeting_id: {}",
                meeting_id
            );
            Err("Meeting not found or can't convert the json".into())
        }
        Err(e) => {
            log_error!("Failed to save meeting summary for {}: {}", meeting_id, e);
            Err(e.to_string())
        }
    }
}

/// Gets summary status and data (Native SQLx implementation)
///
/// Returns summary status (pending/processing/completed/failed) and parsed result data
#[tauri::command]
pub async fn api_get_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    _auth_token: Option<String>,
) -> Result<SummaryResponse, String> {
    log_info!(
        "api_get_summary (native) called for meeting_id: {}",
        meeting_id
    );
    let pool = state.db_manager.pool();

    match SummaryProcessesRepository::get_summary_data_for_meeting(pool, &meeting_id).await {
        Ok(Some(process)) => {
            let status = process.status.to_lowercase();
            let error = process.error;

            // Parse result data if it exists (regardless of status)
            // This allows displaying restored summaries after cancellation or failure
            let data = if let Some(result_str) = process.result {
                match serde_json::from_str::<serde_json::Value>(&result_str) {
                    Ok(parsed) => Some(parsed),
                    Err(e) => {
                        log_error!("Failed to parse summary result JSON: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            // Fetch meeting title from database
            let meeting_name = match MeetingsRepository::get_meeting(pool, &meeting_id).await {
                Ok(Some(meeting_details)) => {
                    log_info!("Fetched meeting title: {}", &meeting_details.title);
                    Some(meeting_details.title)
                }
                Ok(None) => {
                    log_warn!("Meeting not found for meeting_id: {}", meeting_id);
                    None
                }
                Err(e) => {
                    log_error!("Failed to fetch meeting title: {}", e);
                    None
                }
            };

            let response = SummaryResponse {
                status: status.clone(),
                meeting_name,
                meeting_id: meeting_id.clone(),
                start: process.start_time.map(|t| t.to_rfc3339()),
                end: process.end_time.map(|t| t.to_rfc3339()),
                data,
                error,
            };

            log_info!(
                "Summary status for {}: {}, has_data: {}, meeting_name: {:?}",
                meeting_id,
                status,
                response.data.is_some(),
                response.meeting_name
            );
            Ok(response)
        }
        Ok(None) => {
            log_info!("No summary process found for meeting_id: {}", meeting_id);

            // Still fetch meeting title for idle state
            let meeting_name = match MeetingsRepository::get_meeting(pool, &meeting_id).await {
                Ok(Some(meeting_details)) => Some(meeting_details.title),
                _ => None,
            };

            Ok(SummaryResponse {
                status: "idle".to_string(),
                meeting_name,
                meeting_id,
                start: None,
                end: None,
                data: None,
                error: None,
            })
        }
        Err(e) => {
            log_error!("Error retrieving summary for {}: {}", meeting_id, e);
            Err(format!("Failed to retrieve summary: {}", e))
        }
    }
}

/// Processes transcript and generates summary (Native SQLx implementation)
///
/// Spawns a background task and returns immediately with process_id
#[tauri::command]
pub async fn api_process_transcript<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    text: String,
    model: String,
    model_name: String,
    meeting_id: Option<String>,
    _chunk_size: Option<i32>,
    _overlap: Option<i32>,
    custom_prompt: Option<String>,
    template_id: Option<String>,
    _auth_token: Option<String>,
) -> Result<ProcessTranscriptResponse, String> {
    use uuid::Uuid;

    let m_id = meeting_id.unwrap_or_else(|| format!("meeting-{}", Uuid::new_v4()));
    log_info!(
        "api_process_transcript (native) called for meeting_id: {}, model: {}",
        &m_id,
        &model
    );

    let pool = state.db_manager.pool().clone();
    let final_prompt = custom_prompt.unwrap_or_else(|| "".to_string());
    let final_template_id = template_id.unwrap_or_else(|| "daily_standup".to_string());

    // Create or reset the process entry in the database
    SummaryProcessesRepository::create_or_reset_process(&pool, &m_id)
        .await
        .map_err(|e| format!("Failed to initialize process: {}", e))?;

    log_info!("Summary process initialized for meeting_id: {}", &m_id);

    // Save transcript chunks data (matching Python backend behavior)
    let chunk_size = _chunk_size.unwrap_or(40000);
    let overlap = _overlap.unwrap_or(1000);

    TranscriptChunksRepository::save_transcript_data(
        &pool,
        &m_id,
        &text,
        &model,
        &model_name,
        chunk_size,
        overlap,
    )
    .await
    .map_err(|e| format!("Failed to save transcript data: {}", e))?;

    log_info!("Transcript chunks saved for meeting_id: {}", &m_id);

    // Spawn background task for actual processing
    // Wrapped in catch_unwind to prevent panics from crashing the entire Tauri app
    let meeting_id_clone = m_id.clone();
    let pool_for_panic = pool.clone();
    tauri::async_runtime::spawn(async move {
        let meeting_id_for_panic = meeting_id_clone.clone();

        let task = std::panic::AssertUnwindSafe(
            SummaryService::process_transcript_background(
                app,
                pool,
                meeting_id_clone.clone(),
                text,
                model,
                model_name,
                final_prompt,
                final_template_id,
            )
        );

        match task.catch_unwind().await {
            Ok(()) => {
                // Normal completion (success or handled error within the service)
            }
            Err(panic_info) => {
                // A panic occurred in the background task - catch it to prevent app crash
                let panic_msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic during summary generation".to_string()
                };
                log_error!(
                    "PANIC caught in summary background task for meeting_id {}: {}",
                    meeting_id_for_panic,
                    panic_msg
                );
                // Update the database with the error so the frontend polling picks it up
                if let Err(e) = SummaryProcessesRepository::update_process_failed(
                    &pool_for_panic,
                    &meeting_id_for_panic,
                    &format!("Internal error: {}", panic_msg),
                ).await {
                    log_error!("Failed to update DB after panic for {}: {}", meeting_id_for_panic, e);
                }
            }
        }
    });

    log_info!("Background task spawned for meeting_id: {}", &m_id);

    Ok(ProcessTranscriptResponse {
        message: "Summary generation started".to_string(),
        process_id: m_id,
    })
}

/// Migrates all existing meeting summaries by validating and sanitising
/// BlockNote JSON blocks (including `children`) to prevent the
/// "Invalid array passed to renderSpec" ProseMirror error.
///
/// For each meeting that has a `summary_json` array, the command:
/// 1. Parses the stored JSON.
/// 2. Removes blocks whose `content` is a plain string (legacy format).
/// 3. Removes blocks whose `content` items are not valid inline-content objects.
/// 4. Recursively validates and sanitises `children` arrays.
/// 5. Saves the cleaned summary back to the database.
///
/// Returns a JSON object with `{ migrated, skipped, errors }` counts.
#[tauri::command]
pub async fn migrate_summaries<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    log_info!("migrate_summaries: starting migration of all meeting summaries");
    let pool = state.db_manager.pool();

    // Fetch all summary_processes rows that have a result
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT meeting_id, result FROM summary_processes WHERE result IS NOT NULL AND result != ''",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Failed to fetch summaries: {}", e))?;

    let total = rows.len();
    let mut migrated: usize = 0;
    let mut skipped: usize = 0;
    let mut errors: usize = 0;

    for (meeting_id, result_str) in rows {
        // Parse the stored JSON
        let parsed: serde_json::Value = match serde_json::from_str(&result_str) {
            Ok(v) => v,
            Err(e) => {
                log_warn!(
                    "migrate_summaries: failed to parse JSON for meeting {}: {}",
                    meeting_id,
                    e
                );
                errors += 1;
                continue;
            }
        };

        // Only process objects that contain summary_json
        let summary_json = match parsed.get("summary_json") {
            Some(serde_json::Value::Array(arr)) if !arr.is_empty() => arr.clone(),
            _ => {
                skipped += 1;
                continue;
            }
        };

        // Sanitise blocks recursively
        let sanitized: Vec<serde_json::Value> = summary_json
            .into_iter()
            .filter_map(|block| sanitize_blocknote_block(block))
            .collect();

        if sanitized.len() == 0 {
            log_warn!(
                "migrate_summaries: all blocks invalid for meeting {}, skipping save",
                meeting_id
            );
            errors += 1;
            continue;
        }

        // Build updated data object
        let mut updated = parsed.clone();
        if let Some(obj) = updated.as_object_mut() {
            obj.insert(
                "summary_json".to_string(),
                serde_json::Value::Array(sanitized),
            );
        }

        // Persist back to database
        match SummaryProcessesRepository::update_meeting_summary(pool, &meeting_id, &updated).await {
            Ok(true) => {
                log_info!("migrate_summaries: migrated meeting {}", meeting_id);
                migrated += 1;
            }
            Ok(false) => {
                log_warn!("migrate_summaries: meeting {} not found during save", meeting_id);
                errors += 1;
            }
            Err(e) => {
                log_error!(
                    "migrate_summaries: failed to save meeting {}: {}",
                    meeting_id,
                    e
                );
                errors += 1;
            }
        }
    }

    log_info!(
        "migrate_summaries: complete. total={}, migrated={}, skipped={}, errors={}",
        total,
        migrated,
        skipped,
        errors
    );

    Ok(serde_json::json!({
        "total": total,
        "migrated": migrated,
        "skipped": skipped,
        "errors": errors,
    }))
}

/// Recursively sanitises a single BlockNote block value.
/// Returns `None` if the block is fundamentally invalid and should be dropped.
fn sanitize_blocknote_block(block: serde_json::Value) -> Option<serde_json::Value> {
    let obj = block.as_object()?;

    // Must have a string `type`
    let block_type = obj.get("type")?.as_str()?;
    if block_type.is_empty() {
        return None;
    }

    // `content` must not be a plain string (legacy format)
    if let Some(content_val) = obj.get("content") {
        if content_val.is_string() {
            log_warn!(
                "sanitize_blocknote_block: dropping block '{}' with string content (legacy format)",
                block_type
            );
            return None;
        }

        // Validate inline content items
        if let Some(content_arr) = content_val.as_array() {
            for item in content_arr {
                if !is_valid_inline_content(item) {
                    log_warn!(
                        "sanitize_blocknote_block: block '{}' has invalid inline content item",
                        block_type
                    );
                    // We don't drop the whole block for one bad item;
                    // the TypeScript sanitiser handles granular repair.
                    // Here we drop the block to be safe.
                    return None;
                }
            }
        }
    }

    // Recursively sanitise `children`
    let sanitized_children: Vec<serde_json::Value> = obj
        .get("children")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .cloned()
                .filter_map(sanitize_blocknote_block)
                .collect()
        })
        .unwrap_or_default();

    // Rebuild the block with sanitised children
    let mut new_obj = obj.clone();
    new_obj.insert(
        "children".to_string(),
        serde_json::Value::Array(sanitized_children),
    );

    Some(serde_json::Value::Object(new_obj))
}

/// Validates a single inline-content item (mirrors TypeScript isValidInlineContent).
fn is_valid_inline_content(item: &serde_json::Value) -> bool {
    let obj = match item.as_object() {
        Some(o) => o,
        None => return false,
    };

    // Must have a string `type`
    let item_type = match obj.get("type").and_then(|t| t.as_str()) {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };

    // Text nodes must have a string `text`
    if item_type == "text" {
        if !obj.get("text").map(|t| t.is_string()).unwrap_or(false) {
            return false;
        }
    }

    // `styles` must be an object when present
    if let Some(styles) = obj.get("styles") {
        if !styles.is_null() && !styles.is_object() {
            return false;
        }
    }

    true
}

/// Cancels an ongoing summary generation process
///
/// This command triggers the cancellation token for the specified meeting,
/// stopping the summary generation gracefully.
#[tauri::command]
pub async fn api_cancel_summary<R: Runtime>(
    _app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<serde_json::Value, String> {
    log_info!("api_cancel_summary called for meeting_id: {}", meeting_id);

    // Trigger cancellation via the service
    let cancelled = SummaryService::cancel_summary(&meeting_id);

    if cancelled {
        // Update database status to cancelled
        let pool = state.db_manager.pool();
        if let Err(e) = SummaryProcessesRepository::update_process_cancelled(pool, &meeting_id).await {
            log_error!("Failed to update DB status to cancelled for {}: {}", meeting_id, e);
            return Err(format!("Failed to update cancellation status: {}", e));
        }

        log_info!("Successfully cancelled summary generation for meeting_id: {}", meeting_id);
        Ok(serde_json::json!({
            "message": "Summary generation cancelled successfully",
            "meeting_id": meeting_id,
        }))
    } else {
        log_warn!("No active summary generation found for meeting_id: {}", meeting_id);
        Ok(serde_json::json!({
            "message": "No active summary generation to cancel",
            "meeting_id": meeting_id,
        }))
    }
}
