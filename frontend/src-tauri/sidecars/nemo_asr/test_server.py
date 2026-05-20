#!/usr/bin/env python3
"""Tests for the NeMo ASR sidecar server.

Run with: python -m pytest test_server.py -v

Note: These tests mock NeMo/torch dependencies so they can run without
the heavy NeMo installation. Integration tests with real models require
the full NeMo environment.
"""

from __future__ import annotations

import io
import json
import os
import tempfile
from pathlib import Path
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(autouse=True)
def _reset_globals():
    """Reset global state between tests."""
    import server

    server._current_model = None
    server._current_model_id = None
    server._download_tasks.clear()
    yield
    server._current_model = None
    server._current_model_id = None
    server._download_tasks.clear()


@pytest.fixture
def models_dir(tmp_path: Path):
    """Create a temporary models directory."""
    import server

    original = server.MODELS_ROOT
    server.MODELS_ROOT = tmp_path
    yield tmp_path
    server.MODELS_ROOT = original


@pytest.fixture
def client(models_dir):
    """Create a test client for the FastAPI app."""
    from fastapi.testclient import TestClient
    from server import app

    with TestClient(app) as c:
        yield c


# ---------------------------------------------------------------------------
# Health endpoint tests
# ---------------------------------------------------------------------------


class TestHealth:
    def test_health_returns_ok(self, client):
        resp = client.get("/health")
        assert resp.status_code == 200
        data = resp.json()
        assert data["status"] == "ok"
        assert "device" in data

    def test_health_shows_no_model_loaded(self, client):
        resp = client.get("/health")
        data = resp.json()
        assert data["model_loaded"] is None

    def test_health_shows_loaded_model(self, client):
        import server

        server._current_model = MagicMock()
        server._current_model_id = "nvidia/parakeet-rnnt-1.1b"

        resp = client.get("/health")
        data = resp.json()
        assert data["model_loaded"] == "nvidia/parakeet-rnnt-1.1b"


# ---------------------------------------------------------------------------
# Models endpoint tests
# ---------------------------------------------------------------------------


class TestModels:
    def test_models_empty_when_no_downloads(self, client, models_dir):
        resp = client.get("/models")
        assert resp.status_code == 200
        assert resp.json() == []

    def test_models_lists_downloaded_nemo_file(self, client, models_dir):
        # Create a fake downloaded model
        model_dir = models_dir / "nvidia--parakeet-rnnt-1.1b"
        model_dir.mkdir()
        nemo_file = model_dir / "parakeet-rnnt-1.1b.nemo"
        nemo_file.write_bytes(b"fake-nemo-content")

        resp = client.get("/models")
        assert resp.status_code == 200
        data = resp.json()
        assert len(data) == 1
        assert data[0]["model_id"] == "nvidia/parakeet-rnnt-1.1b"
        assert data[0]["filename"] == "parakeet-rnnt-1.1b.nemo"
        assert data[0]["ready"] is True


# ---------------------------------------------------------------------------
# Transcribe endpoint tests
# ---------------------------------------------------------------------------


class TestTranscribe:
    def test_transcribe_rejects_when_no_model_loaded(self, client):
        # Create a valid WAV file
        wav_bytes = self._make_wav_bytes(16000, 1.0)
        resp = client.post(
            "/transcribe",
            files={"audio": ("test.wav", wav_bytes, "audio/wav")},
        )
        assert resp.status_code == 400
        assert "No model loaded" in resp.json()["detail"]

    def test_transcribe_rejects_empty_audio(self, client):
        import server

        server._current_model = MagicMock()
        server._current_model_id = "test-model"

        resp = client.post(
            "/transcribe",
            files={"audio": ("test.wav", b"", "audio/wav")},
        )
        assert resp.status_code == 400

    def test_transcribe_rejects_wrong_sample_rate(self, client):
        import server

        server._current_model = MagicMock()
        server._current_model_id = "test-model"

        # Create a 44100 Hz WAV
        wav_bytes = self._make_wav_bytes(44100, 0.5)
        resp = client.post(
            "/transcribe",
            files={"audio": ("test.wav", wav_bytes, "audio/wav")},
        )
        assert resp.status_code == 400
        assert "16 kHz" in resp.json()["detail"] or "16000" in resp.json()["detail"]

    def test_transcribe_success_with_valid_audio(self, client):
        import server

        # Mock the NeMo model
        mock_model = MagicMock()
        mock_model.transcribe.return_value = ["Hello world"]
        server._current_model = mock_model
        server._current_model_id = "test-model"

        wav_bytes = self._make_wav_bytes(16000, 1.0)
        resp = client.post(
            "/transcribe",
            files={"audio": ("test.wav", wav_bytes, "audio/wav")},
        )
        assert resp.status_code == 200
        data = resp.json()
        assert data["text"] == "Hello world"

    @staticmethod
    def _make_wav_bytes(sample_rate: int, duration: float) -> bytes:
        """Create a minimal WAV file in memory."""
        import numpy as np
        import soundfile as sf

        samples = int(sample_rate * duration)
        audio = np.zeros(samples, dtype=np.float32)
        buf = io.BytesIO()
        sf.write(buf, audio, sample_rate, format="WAV", subtype="PCM_16")
        buf.seek(0)
        return buf.read()


# ---------------------------------------------------------------------------
# Load / Unload endpoint tests
# ---------------------------------------------------------------------------


class TestLoadUnload:
    def test_unload_when_no_model(self, client):
        resp = client.post("/unload")
        assert resp.status_code == 200
        assert resp.json()["status"] == "no_model_loaded"

    def test_load_missing_model_returns_404(self, client, models_dir):
        resp = client.post(
            "/load",
            json={"model_id": "nonexistent/model"},
        )
        assert resp.status_code == 404


# ---------------------------------------------------------------------------
# Download endpoint tests
# ---------------------------------------------------------------------------


class TestDownload:
    def test_download_already_exists(self, client, models_dir):
        # Create a fake existing model
        model_dir = models_dir / "nvidia--parakeet-rnnt-1.1b"
        model_dir.mkdir()
        nemo_file = model_dir / "parakeet-rnnt-1.1b.nemo"
        nemo_file.write_bytes(b"fake-content")

        resp = client.post(
            "/download",
            json={
                "repo_id": "nvidia/parakeet-rnnt-1.1b",
                "filename": "parakeet-rnnt-1.1b.nemo",
            },
        )
        assert resp.status_code == 200
        assert resp.json()["status"] == "already_downloaded"


# ---------------------------------------------------------------------------
# Helper function tests
# ---------------------------------------------------------------------------


class TestHelpers:
    def test_model_dir_replaces_slash(self):
        from server import _model_dir

        result = _model_dir("nvidia/parakeet-rnnt-1.1b")
        assert "--" in str(result)
        assert "/" not in result.name

    def test_find_nemo_file_returns_none_for_missing_dir(self):
        from server import _find_nemo_file

        result = _find_nemo_file(Path("/nonexistent/path"))
        assert result is None

    def test_find_nemo_file_finds_nemo_extension(self, tmp_path):
        from server import _find_nemo_file

        nemo_file = tmp_path / "model.nemo"
        nemo_file.write_bytes(b"content")
        (tmp_path / "other.txt").write_bytes(b"text")

        result = _find_nemo_file(tmp_path)
        assert result is not None
        assert result.suffix == ".nemo"
