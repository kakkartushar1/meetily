#!/usr/bin/env python3
"""Local NeMo ASR sidecar for Meetily.

The Rust app owns download progress and process lifecycle. This sidecar keeps
the Python/PyTorch/NeMo dependency stack out of the main Tauri binary and
exposes a small localhost API for loading `.nemo` checkpoints and transcribing
16 kHz mono WAV files.
"""

from __future__ import annotations

import argparse
import logging
from pathlib import Path
from typing import Any

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
import uvicorn

logger = logging.getLogger("meetily-nemo-asr")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

app = FastAPI(title="Meetily NeMo ASR Sidecar", version="0.1.0")

_model: Any | None = None
_model_id: str | None = None
_device: str = "cpu"


class LoadRequest(BaseModel):
    model_id: str
    model_path: str


class TranscribeRequest(BaseModel):
    audio_path: str


@app.get("/health")
def health() -> dict[str, Any]:
    return {
        "status": "ok",
        "model_id": _model_id,
        "device": _device,
        "loaded": _model is not None,
    }


@app.post("/load")
def load_model(request: LoadRequest) -> dict[str, str]:
    global _model, _model_id, _device

    model_path = Path(request.model_path)
    if not model_path.exists():
        raise HTTPException(status_code=404, detail=f"Model file not found: {model_path}")

    try:
        import torch
        from nemo.collections.asr.models import ASRModel
    except Exception as exc:
        raise HTTPException(
            status_code=500,
            detail=(
                "NeMo ASR dependencies are not installed. Install requirements-nemo.txt "
                f"for the sidecar Python environment. Import error: {exc}"
            ),
        ) from exc

    if _model is not None and _model_id == request.model_id:
        return {"status": "loaded", "model_id": request.model_id}

    device = "cuda" if torch.cuda.is_available() else "cpu"
    logger.info("Loading NeMo ASR model %s on %s from %s", request.model_id, device, model_path)

    model = ASRModel.restore_from(restore_path=str(model_path), map_location=device)
    model.to(device)
    model.eval()

    _model = model
    _model_id = request.model_id
    _device = device
    return {"status": "loaded", "model_id": request.model_id}


@app.post("/transcribe")
def transcribe(request: TranscribeRequest) -> dict[str, str]:
    if _model is None:
        raise HTTPException(status_code=409, detail="No NeMo model loaded")

    audio_path = Path(request.audio_path)
    if not audio_path.exists():
        raise HTTPException(status_code=404, detail=f"Audio file not found: {audio_path}")

    try:
        result = _model.transcribe([str(audio_path)], batch_size=1)
    except Exception as exc:
        raise HTTPException(status_code=500, detail=f"NeMo transcription failed: {exc}") from exc

    return {"text": _extract_text(result)}


@app.post("/unload")
def unload() -> dict[str, str]:
    global _model, _model_id
    _model = None
    _model_id = None
    return {"status": "unloaded"}


def _extract_text(result: Any) -> str:
    """Normalize NeMo transcribe output across toolkit versions."""
    if result is None:
        return ""
    if isinstance(result, str):
        return result
    if isinstance(result, (list, tuple)):
        if not result:
            return ""
        return _extract_text(result[0])
    text = getattr(result, "text", None)
    if text is not None:
        return str(text)
    return str(result)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=5877)
    parser.add_argument("--models-dir", default=None)
    args = parser.parse_args()

    if args.models_dir:
        Path(args.models_dir).mkdir(parents=True, exist_ok=True)

    uvicorn.run(app, host="127.0.0.1", port=args.port, log_level="info")


if __name__ == "__main__":
    main()
