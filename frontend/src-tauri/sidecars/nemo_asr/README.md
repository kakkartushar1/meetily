# Meetily NeMo ASR Sidecar

This optional sidecar runs NVIDIA NeMo `.nemo` ASR checkpoints for local
transcription. It is lazy-started by the Tauri app only when a `.nemo`
transcription model is selected.

Install dependencies into the Python environment used by `MEETILY_NEMO_PYTHON`:

```powershell
python -m pip install -r frontend/src-tauri/sidecars/nemo_asr/requirements-nemo.txt
```

Useful environment variables:

- `MEETILY_NEMO_PYTHON`: Python executable to spawn.
- `MEETILY_NEMO_SIDECAR`: absolute path to `server.py`.
- `MEETILY_NEMO_PORT`: localhost port, default `5877`.

The sidecar API is local-only:

- `GET /health`
- `POST /load`
- `POST /transcribe`
- `POST /unload`
