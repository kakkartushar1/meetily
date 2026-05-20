#!/usr/bin/env python3
"""
NeMo ASR Sidecar – lightweight local HTTP service for .nemo model inference.

This sidecar is lazy-started by the Rust host only when a .nemo model is
selected or downloaded.  It exposes a small REST API consumed exclusively
by the Tauri backend over localhost.

Endpoints
---------
GET  /health          – liveness probe
GET  /models          – list locally available .nemo models
POST /download        – download a model from HuggingFace
POST /load            – load a model into memory
POST /transcribe      – transcribe 16 kHz mono WAV audio
POST /unload          – release model from memory
"""

from __future__ import annotations

import asyncio
import io
import logging
import os
import signal
import sys
import tempfile
import time
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Optional

import soundfile as sf
import torch
import uvicorn
from fastapi import FastAPI, HTTPException, UploadFile, File, Form
from fastapi.responses import JSONResponse
from pydantic import BaseModel

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
)
logger = logging.getLogger("nemo_asr")

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
DEFAULT_PORT = 9876
MODELS_ROOT = Path(
    os.environ.get(
        "NEMO_MODELS_DIR",
        Path.home() / ".meetily" / "models" / "nemo",
    )
)

# ---------------------------------------------------------------------------
# GPU / device detection
# ---------------------------------------------------------------------------

def _detect_device() -> str:
    """Return 'cuda' if a usable GPU is found, otherwise 'cpu'."""
    if torch.cuda.is_available():
        gpu_name = torch.cuda.get_device_name(0)
        logger.info("GPU detected: %s – using CUDA", gpu_name)
        return "cuda"
    logger.warning(
        "No CUDA GPU detected – falling back to CPU. "
        "Transcription will be significantly slower."
    )
    return "cpu"


DEVICE = _detect_device()

# ---------------------------------------------------------------------------
# Global model state
# ---------------------------------------------------------------------------

_current_model: Optional[object] = None  # nemo ASR model instance
_current_model_id: Optional[str] = None
_model_lock = asyncio.Lock()
_download_tasks: dict[str, asyncio.Task] = {}


# ---------------------------------------------------------------------------
# Pydantic request / response schemas
# ---------------------------------------------------------------------------

class DownloadRequest(BaseModel):
    repo_id: str
    filename: str


class LoadRequest(BaseModel):
    model_id: str


class TranscribeResponse(BaseModel):
    text: str


class ModelInfo(BaseModel):
    model_id: str
    filename: str
    size_bytes: int
    ready: bool


class HealthResponse(BaseModel):
    status: str
    device: str
    model_loaded: Optional[str]


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _model_dir(repo_id: str) -> Path:
    """Return the local directory for a given HF repo_id."""
    safe_name = repo_id.replace("/", "--")
    return MODELS_ROOT / safe_name


def _find_nemo_file(model_dir: Path) -> Optional[Path]:
    """Find the first .nemo file in a model directory."""
    if not model_dir.exists():
        return None
    for f in model_dir.iterdir():
        if f.suffix == ".nemo":
            return f
    return None


def _list_local_models() -> list[ModelInfo]:
    """Scan MODELS_ROOT for downloaded .nemo models."""
    models: list[ModelInfo] = []
    if not MODELS_ROOT.exists():
        return models
    for d in sorted(MODELS_ROOT.iterdir()):
        if not d.is_dir():
            continue
        nemo_file = _find_nemo_file(d)
        if nemo_file is None:
            continue
        repo_id = d.name.replace("--", "/")
        models.append(
            ModelInfo(
                model_id=repo_id,
                filename=nemo_file.name,
                size_bytes=nemo_file.stat().st_size,
                ready=True,
            )
        )
    return models


async def _download_model_async(repo_id: str, filename: str) -> Path:
    """Download a .nemo model from HuggingFace Hub (resumable)."""
    from huggingface_hub import hf_hub_download

    dest_dir = _model_dir(repo_id)
    dest_dir.mkdir(parents=True, exist_ok=True)

    logger.info("Downloading %s/%s to %s ...", repo_id, filename, dest_dir)

    # Run blocking download in a thread pool
    loop = asyncio.get_running_loop()
    downloaded_path = await loop.run_in_executor(
        None,
        lambda: hf_hub_download(
            repo_id=repo_id,
            filename=filename,
            local_dir=str(dest_dir),
            local_dir_use_symlinks=False,
            resume_download=True,
        ),
    )

    logger.info("Download complete: %s", downloaded_path)
    return Path(downloaded_path)


# ---------------------------------------------------------------------------
# FastAPI app
# ---------------------------------------------------------------------------

@asynccontextmanager
async def lifespan(app: FastAPI):
    """Startup / shutdown lifecycle."""
    logger.info("NeMo ASR sidecar starting on device=%s", DEVICE)
    MODELS_ROOT.mkdir(parents=True, exist_ok=True)
    yield
    # Cleanup: unload model on shutdown
    global _current_model, _current_model_id
    if _current_model is not None:
        logger.info("Unloading model on shutdown")
        del _current_model
        _current_model = None
        _current_model_id = None
    # Cancel pending downloads
    for task in _download_tasks.values():
        task.cancel()
    _download_tasks.clear()


app = FastAPI(
    title="NeMo ASR Sidecar",
    version="0.1.0",
    lifespan=lifespan,
)


# ---------------------------------------------------------------------------
# Endpoints
# ---------------------------------------------------------------------------

@app.get("/health", response_model=HealthResponse)
async def health():
    """Liveness probe."""
    return HealthResponse(
        status="ok",
        device=DEVICE,
        model_loaded=_current_model_id,
    )


@app.get("/models", response_model=list[ModelInfo])
async def list_models():
    """List locally available .nemo models."""
    return _list_local_models()


@app.post("/download")
async def download_model(req: DownloadRequest):
    """Download a .nemo model from HuggingFace (resumable)."""
    task_key = f"{req.repo_id}/{req.filename}"

    # Prevent duplicate downloads
    if task_key in _download_tasks and not _download_tasks[task_key].done():
        return JSONResponse(
            status_code=409,
            content={"error": "Download already in progress", "repo_id": req.repo_id},
        )

    # Check if already downloaded
    model_dir = _model_dir(req.repo_id)
    existing = model_dir / req.filename
    if existing.exists() and existing.stat().st_size > 0:
        return {
            "status": "already_downloaded",
            "path": str(existing),
            "size_bytes": existing.stat().st_size,
        }

    try:
        path = await _download_model_async(req.repo_id, req.filename)
        return {
            "status": "downloaded",
            "path": str(path),
            "size_bytes": path.stat().st_size,
        }
    except Exception as exc:
        logger.exception("Download failed for %s", task_key)
        raise HTTPException(status_code=500, detail=str(exc)) from exc


@app.post("/load")
async def load_model(req: LoadRequest):
    """Load a .nemo model into memory for transcription."""
    global _current_model, _current_model_id

    async with _model_lock:
        # Already loaded?
        if _current_model_id == req.model_id and _current_model is not None:
            return {"status": "already_loaded", "model_id": req.model_id}

        # Find the .nemo file
        model_dir = _model_dir(req.model_id)
        nemo_file = _find_nemo_file(model_dir)
        if nemo_file is None:
            raise HTTPException(
                status_code=404,
                detail=f"Model '{req.model_id}' not found locally. Download it first.",
            )

        # Unload previous model
        if _current_model is not None:
            logger.info("Unloading previous model: %s", _current_model_id)
            del _current_model
            _current_model = None
            _current_model_id = None
            if DEVICE == "cuda":
                torch.cuda.empty_cache()

        # Load new model
        logger.info("Loading NeMo model from %s on %s ...", nemo_file, DEVICE)
        try:
            import nemo.collections.asr as nemo_asr

            loop = asyncio.get_running_loop()
            model = await loop.run_in_executor(
                None,
                lambda: nemo_asr.models.ASRModel.restore_from(str(nemo_file), map_location=DEVICE),
            )
            model.eval()
            if DEVICE == "cuda":
                model = model.cuda()

            _current_model = model
            _current_model_id = req.model_id
            logger.info("Model loaded successfully: %s", req.model_id)
            return {"status": "loaded", "model_id": req.model_id, "device": DEVICE}
        except Exception as exc:
            logger.exception("Failed to load model %s", req.model_id)
            raise HTTPException(status_code=500, detail=str(exc)) from exc


@app.post("/transcribe", response_model=TranscribeResponse)
async def transcribe(
    audio: UploadFile = File(...),
    temp_path: Optional[str] = Form(None),
):
    """Transcribe 16 kHz mono WAV audio.

    Accepts either:
    - An uploaded WAV file via multipart form
    - A local temp file path via the `temp_path` form field
    """
    if _current_model is None:
        raise HTTPException(
            status_code=400,
            detail="No model loaded. Call POST /load first.",
        )

    wav_path: Optional[Path] = None
    cleanup_temp = False

    try:
        if temp_path and Path(temp_path).exists():
            wav_path = Path(temp_path)
        elif audio and audio.filename:
            # Read uploaded bytes and write to temp file
            content = await audio.read()
            if len(content) == 0:
                raise HTTPException(status_code=400, detail="Empty audio file")

            # Validate audio format
            try:
                data, sr = sf.read(io.BytesIO(content))
            except Exception as exc:
                raise HTTPException(
                    status_code=400,
                    detail=f"Invalid audio format: {exc}",
                ) from exc

            if sr != 16000:
                raise HTTPException(
                    status_code=400,
                    detail=f"Expected 16 kHz sample rate, got {sr} Hz. "
                           f"Please resample to 16000 Hz before sending.",
                )

            # Write to temp file for NeMo
            tmp = tempfile.NamedTemporaryFile(suffix=".wav", delete=False)
            sf.write(tmp.name, data, 16000, format="WAV", subtype="PCM_16")
            wav_path = Path(tmp.name)
            cleanup_temp = True
        else:
            raise HTTPException(
                status_code=400,
                detail="Provide either an audio file upload or a temp_path.",
            )

        # Transcribe
        logger.info("Transcribing %s ...", wav_path)
        start = time.monotonic()

        loop = asyncio.get_running_loop()
        transcriptions = await loop.run_in_executor(
            None,
            lambda: _current_model.transcribe([str(wav_path)]),
        )

        # NeMo returns list of strings or list of Hypothesis objects
        if isinstance(transcriptions, (list, tuple)):
            # Handle both old and new NeMo API
            if len(transcriptions) > 0:
                first = transcriptions[0]
                if isinstance(first, str):
                    text = first
                elif isinstance(first, (list, tuple)) and len(first) > 0:
                    # Some NeMo versions return [[text]]
                    text = str(first[0])
                elif hasattr(first, "text"):
                    text = first.text
                else:
                    text = str(first)
            else:
                text = ""
        else:
            text = str(transcriptions)

        elapsed = time.monotonic() - start
        logger.info("Transcription complete in %.2fs: '%s'", elapsed, text[:100])

        return TranscribeResponse(text=text.strip())

    finally:
        if cleanup_temp and wav_path and wav_path.exists():
            try:
                wav_path.unlink()
            except OSError:
                pass


@app.post("/unload")
async def unload_model():
    """Unload the current model to free GPU/CPU memory."""
    global _current_model, _current_model_id

    async with _model_lock:
        if _current_model is None:
            return {"status": "no_model_loaded"}

        model_id = _current_model_id
        logger.info("Unloading model: %s", model_id)
        del _current_model
        _current_model = None
        _current_model_id = None

        if DEVICE == "cuda":
            torch.cuda.empty_cache()

        return {"status": "unloaded", "model_id": model_id}


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main():
    """Run the sidecar HTTP server."""
    port = int(os.environ.get("NEMO_ASR_PORT", DEFAULT_PORT))
    host = os.environ.get("NEMO_ASR_HOST", "127.0.0.1")

    logger.info("Starting NeMo ASR sidecar on %s:%d", host, port)
    logger.info("Models directory: %s", MODELS_ROOT)
    logger.info("Device: %s", DEVICE)

    uvicorn.run(
        app,
        host=host,
        port=port,
        log_level="info",
        access_log=False,  # Reduce noise; Rust host logs requests
    )


if __name__ == "__main__":
    main()
