use axum::{
    extract::{State, Json},
    http::{StatusCode, HeaderMap, header},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize};
use std::{collections::HashMap, net::SocketAddr, sync::{Arc, Mutex}, path::PathBuf};
use tracing::{info, error};
use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tokio::io::AsyncWriteExt;
use sha2::{Sha256, Digest};
use std::time::{SystemTime, Duration};

mod helper;
use helper::{TextToSpeech, Style, load_text_to_speech, load_voice_style};

// ============================================================================
// Configuration & State
// ============================================================================

struct AppState {
    tts: Arc<Mutex<TextToSpeech>>,
    voice_styles: HashMap<String, Style>,
    cache_dir: PathBuf,
}

#[derive(Deserialize, Debug)]
struct CreateSpeechRequest {
    model: Option<String>,
    input: String,
    voice: String,
    response_format: Option<String>, // mp3, opus, aac, flac, wav, pcm
    speed: Option<f32>,
    // Supertonic specific fields
    total_step: Option<usize>,
    lang: Option<String>,
}

// ============================================================================
// Main Server
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Initializing Supertonic OpenAI TTS Server...");

    // Load TTS
    let onnx_dir = "assets/onnx";
    let tts = load_text_to_speech(onnx_dir, false)?;
    info!("Loaded TTS models from {}", onnx_dir);

    // Load Voice Styles
    let voice_style_dir = "assets/voice_styles";
    let mut voice_styles = HashMap::new();
    
    // Default mapping for OpenAI voice names
    let openai_mapping = vec![
        ("Alex", "M1"), ("James", "M2"), ("Robert", "M3"), ("Sam", "M4"), ("Daniel", "M5"),
        ("Sarah", "F1"), ("Lily", "F2"), ("Jessica", "F3"), ("Olivia", "F4"), ("Emily", "F5"),
    ];

    // Load all JSON files in voice_style_dir
    let paths: Vec<PathBuf> = std::fs::read_dir(voice_style_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().map_or(false, |ext| ext == "json"))
        .collect();

    for path in paths {
        let file_stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let path_str = path.to_string_lossy().to_string();
        
        match load_voice_style(&[path_str], false) {
            Ok(style) => {
                info!("Loaded voice style: {}", file_stem);
                voice_styles.insert(file_stem.clone(), style);
            }
            Err(e) => error!("Failed to load voice style {}: {}", file_stem, e),
        }
    }

    // Apply OpenAI mappings
    for (openai_name, target_style) in &openai_mapping {
        if let Some(style) = voice_styles.get(*target_style) {
            voice_styles.insert(openai_name.to_string(), style.clone());
            info!("Mapped OpenAI voice '{}' to style '{}'", openai_name, target_style);
        }
    }

    // Create cache directory
    let cache_dir = PathBuf::from("cache");
    std::fs::create_dir_all(&cache_dir)?;
    
    // Start cache pruning task
    let cache_dir_clone = cache_dir.clone();
    tokio::spawn(async move {
        prune_cache_task(cache_dir_clone).await;
    });

    let app_state = Arc::new(AppState {
        tts: Arc::new(Mutex::new(tts)),
        voice_styles,
        cache_dir,
    });

    let app = Router::new()
        .route("/v1/audio/speech", post(create_speech))
        .route("/health", get(health_check))
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("Listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> impl IntoResponse {
    StatusCode::OK
}

async fn create_speech(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CreateSpeechRequest>,
) -> Response {
    // Validate input
    if payload.input.is_empty() {
        return (StatusCode::BAD_REQUEST, "Input text cannot be empty").into_response();
    }
    
    // Model check
    if let Some(ref m) = payload.model {
        if m != "supertonic-2" && m != "tts-1" {
             info!("Received request for model '{}', using supertonic-2", m);
        }
    }

    let voice_name = payload.voice.clone();
    if !state.voice_styles.contains_key(&voice_name) {
         return (StatusCode::BAD_REQUEST, format!("Unsupported voice: {}", voice_name)).into_response();
    }
    
    // Validate total_step
    let total_step = payload.total_step.unwrap_or(5);
    if total_step < 1 || total_step > 10 {
        return (StatusCode::BAD_REQUEST, "total_step must be between 1 and 10").into_response();
    }
    
    // Parse Languages
    let lang_str = payload.lang.clone().unwrap_or_else(|| "en".to_string());
    let valid_langs = ["en", "ko", "es", "pt", "fr"];
    let langs: Vec<String> = lang_str.split(',')
        .map(|s| s.trim().to_string())
        .collect();
        
    for l in &langs {
        if !valid_langs.contains(&l.as_str()) {
            return (StatusCode::BAD_REQUEST, format!("Invalid language: {}. Supported: {:?}", l, valid_langs)).into_response();
        }
    }

    // Parse Input Text segments
    // We check if input contains '|'. If so, we split by it.
    // If not, we treat it as single text (even if langs has multiple, we'll repeat or error? Let's use first lang or repeat langs cyclically?)
    // Standard logic: 
    // If user provides "text1|text2" and "en,es", we map 1-to-1.
    // If user provides "text1" and "en,es", ambiguous. Use "en".
    // If user provides "text1|text2" and "en", use "en" for both.
    
    let input_segments: Vec<String> = if payload.input.contains('|') {
        payload.input.split('|').map(|s| s.trim().to_string()).collect()
    } else {
        vec![payload.input.clone()]
    };
    
    // Align langs to input segments
    let mut aligned_langs = Vec::new();
    if langs.len() == 1 {
        // Broadcast single lang to all segments
        for _ in 0..input_segments.len() {
            aligned_langs.push(langs[0].clone());
        }
    } else if langs.len() == input_segments.len() {
        aligned_langs = langs;
    } else {
        return (StatusCode::BAD_REQUEST, format!("Mismatch: Input has {} segments (split by '|'), but {} languages provided. They must match or provide single language.", input_segments.len(), langs.len())).into_response();
    }

    let speed = payload.speed.unwrap_or(1.0);
    let format = payload.response_format.as_deref().unwrap_or("mp3");
    
    // Check cache
    let cache_key = format!("{}:{}:{}:{:.2}:{}:{}", payload.input, voice_name, format, speed, total_step, lang_str);
    let mut hasher = Sha256::new();
    hasher.update(cache_key.as_bytes());
    let hash = hex::encode(hasher.finalize());
    let cache_path = state.cache_dir.join(format!("{}.{}", hash, format));

    if cache_path.exists() {
        info!("Cache hit for {}", hash);
        match tokio::fs::read(&cache_path).await {
            Ok(bytes) => {
                let mut headers = HeaderMap::new();
                headers.insert(header::CONTENT_TYPE, determine_content_type(format).parse().unwrap());
                return (headers, bytes).into_response();
            }
            Err(e) => error!("Failed to read cache: {}", e),
        }
    }

    info!("Generating speech for voice '{}', speed {}, format '{}', steps {}", voice_name, speed, format, total_step);
    
    // Blocking call to TTS
    let tts_arc = state.tts.clone();
    let voice_name_clone = voice_name.clone();
    
    let state_clone = state.clone();

    let generation_result = tokio::task::spawn_blocking(move || {
        let mut tts = tts_arc.lock().unwrap();
        let style = state_clone.voice_styles.get(&voice_name_clone).unwrap();
        
        let mut all_wavs = Vec::new();
        let mut total_dur = 0.0;
        
        // Process each segment
        for (text, lang) in input_segments.iter().zip(aligned_langs.iter()) {
             let (wav, dur) = tts.call(text, lang, style, total_step, speed, 0.3)?;
             all_wavs.extend(wav);
             total_dur += dur;
        }
        
        Ok::<_, anyhow::Error>((all_wavs, total_dur))
    }).await;

    let (wav_samples, _duration) = match generation_result {
        Ok(Ok(res)) => res,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("TTS Error: {}", e)).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Task Error: {}", e)).into_response(),
    };
    
    let sample_rate = {
        let tts = state.tts.lock().unwrap();
        tts.sample_rate
    };

    // Convert to requested format
    let audio_bytes = match convert_audio(&wav_samples, sample_rate, format).await {
        Ok(bytes) => bytes,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Conversion Error: {}", e)).into_response(),
    };

    // Save to cache
    if let Err(e) = tokio::fs::write(&cache_path, &audio_bytes).await {
        error!("Failed to write to cache: {}", e);
    }

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, determine_content_type(format).parse().unwrap());
    (headers, audio_bytes).into_response()
}

fn determine_content_type(format: &str) -> String {
    match format {
        "mp3" => "audio/mpeg",
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "pcm" => "audio/pcm", 
        _ => "application/octet-stream",
    }
    .to_string()
}

async fn convert_audio(samples: &[f32], sample_rate: i32, format: &str) -> Result<Vec<u8>> {
    if format == "pcm" {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            let clamped = sample.max(-1.0).min(1.0);
            let val = (clamped * 32767.0) as i16;
            bytes.extend_from_slice(&val.to_le_bytes());
        }
        return Ok(bytes);
    }

    if format == "wav" {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sample_rate as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = std::io::Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
        for &sample in samples {
            let clamped = sample.max(-1.0).min(1.0);
            let val = (clamped * 32767.0) as i16;
            writer.write_sample(val)?;
        }
        writer.finalize()?;
        return Ok(cursor.into_inner());
    }

    let mut cmd = Command::new("ffmpeg");
    cmd.args(&[
        "-f", "s16le", 
        "-ar", &sample_rate.to_string(),
        "-ac", "1",
        "-i", "pipe:0", 
        "-f", format,   
        "pipe:1"        
    ]);
    
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = cmd.spawn()?;

    let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("Failed to open stdin"))?;
    
    let mut pcm_bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let clamped = sample.max(-1.0).min(1.0);
        let val = (clamped * 32767.0) as i16;
        pcm_bytes.extend_from_slice(&val.to_le_bytes());
    }

    tokio::spawn(async move {
        let _ = stdin.write_all(&pcm_bytes).await;
    });

    let output = child.wait_with_output().await?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!("FFmpeg failed"));
    }

    Ok(output.stdout)
}

async fn prune_cache_task(cache_dir: PathBuf) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Every hour
    loop {
        interval.tick().await;
        info!("Pruning cache...");
        
        let max_age = Duration::from_secs(86400 * 3); // 3 days
        let max_size = 1024 * 1024 * 1024; // 1 GB
        
        let mut files = Vec::new();
        let mut total_size = 0;

        if let Ok(mut entries) = tokio::fs::read_dir(&cache_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Ok(metadata) = entry.metadata().await {
                    if metadata.is_file() {
                        total_size += metadata.len();
                        files.push((entry.path(), metadata.modified().unwrap_or(SystemTime::now()), metadata.len()));
                    }
                }
            }
        }

        files.sort_by(|a, b| a.1.cmp(&b.1));

        let now = SystemTime::now();

        for (path, modified, size) in files {
            let age = now.duration_since(modified).unwrap_or(Duration::from_secs(0));
            let mut remove = false;

            if age > max_age {
                remove = true;
            } else if total_size > max_size {
                remove = true;
            }

            if remove {
                if let Ok(_) = tokio::fs::remove_file(&path).await {
                    total_size -= size;
                    info!("Pruned file: {:?}", path);
                }
            }

            if total_size <= max_size && age <= max_age {
                break;
            }
        }
    }
}
