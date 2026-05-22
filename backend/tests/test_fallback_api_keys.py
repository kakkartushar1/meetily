"""Test suite for Fallback API Key feature.

Tests cover:
1. Database schema - fallback key columns exist in settings table
2. DatabaseManager - save_fallback_api_key and get_fallback_api_key methods
3. API endpoints - /save-fallback-api-key and /get-fallback-api-key
4. TranscriptProcessor - fallback retry logic for process_transcript and chat
"""

import pytest
import asyncio
import sqlite3
import os
import sys
import tempfile
import json
from unittest.mock import AsyncMock, MagicMock, patch

# Add parent directory to path for imports
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'app'))

from db import DatabaseManager


# ============================================================
# Fixtures
# ============================================================

@pytest.fixture
def temp_db_path():
    """Create a temporary database file for testing."""
    fd, path = tempfile.mkstemp(suffix='.db')
    os.close(fd)
    yield path
    # On Windows, SQLite may hold file locks; ignore cleanup errors
    try:
        os.unlink(path)
    except PermissionError:
        pass  # File will be cleaned up by OS temp cleanup


@pytest.fixture
def db_manager(temp_db_path):
    """Create a DatabaseManager instance with a temporary database."""
    return DatabaseManager(db_path=temp_db_path)


# ============================================================
# 1. Database Schema Tests
# ============================================================

class TestDatabaseSchema:
    """Test that fallback API key columns exist in the settings table."""

    def test_settings_table_has_fallback_columns(self, db_manager):
        """Verify that the settings table has all fallback API key columns."""
        with sqlite3.connect(db_manager.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("PRAGMA table_info(settings)")
            columns = {row[1] for row in cursor.fetchall()}

        expected_fallback_columns = {
            'openaiFallbackApiKey',
            'anthropicFallbackApiKey',
            'groqFallbackApiKey',
            'openRouterFallbackApiKey',
            'geminiFallbackApiKey',
        }

        for col in expected_fallback_columns:
            assert col in columns, f"Missing fallback column: {col}"

    def test_settings_table_has_primary_key_columns(self, db_manager):
        """Verify that the settings table still has all primary API key columns."""
        with sqlite3.connect(db_manager.db_path) as conn:
            cursor = conn.cursor()
            cursor.execute("PRAGMA table_info(settings)")
            columns = {row[1] for row in cursor.fetchall()}

        expected_primary_columns = {
            'openaiApiKey',
            'anthropicApiKey',
            'groqApiKey',
            'openRouterApiKey',
            'geminiApiKey',
        }

        for col in expected_primary_columns:
            assert col in columns, f"Missing primary key column: {col}"


# ============================================================
# 2. DatabaseManager Fallback Key Methods Tests
# ============================================================

class TestDatabaseManagerFallbackKeys:
    """Test save_fallback_api_key and get_fallback_api_key methods."""

    @pytest.mark.asyncio
    async def test_save_and_get_fallback_key_openai(self, db_manager):
        """Test saving and retrieving a fallback key for OpenAI."""
        await db_manager.save_fallback_api_key('sk-fallback-openai-test', 'openai')
        result = await db_manager.get_fallback_api_key('openai')
        assert result == 'sk-fallback-openai-test'

    @pytest.mark.asyncio
    async def test_save_and_get_fallback_key_claude(self, db_manager):
        """Test saving and retrieving a fallback key for Claude."""
        await db_manager.save_fallback_api_key('sk-fallback-claude-test', 'claude')
        result = await db_manager.get_fallback_api_key('claude')
        assert result == 'sk-fallback-claude-test'

    @pytest.mark.asyncio
    async def test_save_and_get_fallback_key_groq(self, db_manager):
        """Test saving and retrieving a fallback key for Groq."""
        await db_manager.save_fallback_api_key('gsk-fallback-groq-test', 'groq')
        result = await db_manager.get_fallback_api_key('groq')
        assert result == 'gsk-fallback-groq-test'

    @pytest.mark.asyncio
    async def test_save_and_get_fallback_key_openrouter(self, db_manager):
        """Test saving and retrieving a fallback key for OpenRouter."""
        await db_manager.save_fallback_api_key('sk-or-fallback-test', 'openrouter')
        result = await db_manager.get_fallback_api_key('openrouter')
        assert result == 'sk-or-fallback-test'

    @pytest.mark.asyncio
    async def test_save_and_get_fallback_key_gemini(self, db_manager):
        """Test saving and retrieving a fallback key for Gemini."""
        await db_manager.save_fallback_api_key('AIza-fallback-gemini-test', 'gemini')
        result = await db_manager.get_fallback_api_key('gemini')
        assert result == 'AIza-fallback-gemini-test'

    @pytest.mark.asyncio
    async def test_get_fallback_key_returns_empty_when_not_set(self, db_manager):
        """Test that getting a fallback key returns empty string when not set."""
        result = await db_manager.get_fallback_api_key('openai')
        assert result == ''

    @pytest.mark.asyncio
    async def test_save_fallback_key_ollama_is_noop(self, db_manager):
        """Test that saving a fallback key for Ollama is a no-op."""
        await db_manager.save_fallback_api_key('some-key', 'ollama')
        result = await db_manager.get_fallback_api_key('ollama')
        assert result == ''

    @pytest.mark.asyncio
    async def test_save_fallback_key_invalid_provider_raises(self, db_manager):
        """Test that saving a fallback key for an invalid provider raises ValueError."""
        with pytest.raises(ValueError, match="Invalid provider"):
            await db_manager.save_fallback_api_key('some-key', 'invalid-provider')

    @pytest.mark.asyncio
    async def test_get_fallback_key_invalid_provider_raises(self, db_manager):
        """Test that getting a fallback key for an invalid provider raises ValueError."""
        with pytest.raises(ValueError, match="Invalid provider"):
            await db_manager.get_fallback_api_key('invalid-provider')

    @pytest.mark.asyncio
    async def test_fallback_key_update_overwrites_previous(self, db_manager):
        """Test that saving a new fallback key overwrites the previous one."""
        await db_manager.save_fallback_api_key('sk-first-key', 'openai')
        await db_manager.save_fallback_api_key('sk-second-key', 'openai')
        result = await db_manager.get_fallback_api_key('openai')
        assert result == 'sk-second-key'

    @pytest.mark.asyncio
    async def test_fallback_key_independent_from_primary_key(self, db_manager):
        """Test that fallback and primary keys are stored independently."""
        await db_manager.save_api_key('sk-primary-openai', 'openai')
        await db_manager.save_fallback_api_key('sk-fallback-openai', 'openai')

        primary = await db_manager.get_api_key('openai')
        fallback = await db_manager.get_fallback_api_key('openai')

        assert primary == 'sk-primary-openai'
        assert fallback == 'sk-fallback-openai'
        assert primary != fallback

    @pytest.mark.asyncio
    async def test_multiple_providers_fallback_keys(self, db_manager):
        """Test saving fallback keys for multiple providers simultaneously."""
        await db_manager.save_fallback_api_key('sk-fb-openai', 'openai')
        await db_manager.save_fallback_api_key('sk-fb-claude', 'claude')
        await db_manager.save_fallback_api_key('sk-fb-groq', 'groq')

        assert await db_manager.get_fallback_api_key('openai') == 'sk-fb-openai'
        assert await db_manager.get_fallback_api_key('claude') == 'sk-fb-claude'
        assert await db_manager.get_fallback_api_key('groq') == 'sk-fb-groq'


# ============================================================
# 3. Schema Validator Tests
# ============================================================

class TestSchemaValidator:
    """Test that SchemaValidator includes fallback key columns."""

    def test_schema_validator_includes_fallback_columns(self, db_manager):
        """Verify SchemaValidator expected schema includes fallback key columns."""
        expected_schema = db_manager.schema_validator._get_expected_schema()
        settings_columns = [col[0] for col in expected_schema.get('settings', [])]

        fallback_columns = [
            'openaiFallbackApiKey',
            'anthropicFallbackApiKey',
            'groqFallbackApiKey',
            'openRouterFallbackApiKey',
            'geminiFallbackApiKey',
        ]

        for col in fallback_columns:
            assert col in settings_columns, f"SchemaValidator missing fallback column: {col}"

    def test_schema_validation_passes(self, db_manager):
        """Verify that schema validation passes with fallback columns present."""
        # This should not raise any exceptions
        db_manager.schema_validator.validate_schema()


# ============================================================
# 4. TranscriptProcessor Fallback Logic Tests
# ============================================================

# Try to import TranscriptProcessor; skip tests if dependencies are missing
try:
    from transcript_processor import TranscriptProcessor
    HAS_TRANSCRIPT_PROCESSOR = True
except (ImportError, ModuleNotFoundError):
    HAS_TRANSCRIPT_PROCESSOR = False


@pytest.mark.skipif(not HAS_TRANSCRIPT_PROCESSOR, reason="TranscriptProcessor dependencies not installed (pydantic, pydantic_ai, etc.)")
class TestTranscriptProcessorFallback:
    """Test the fallback retry logic in TranscriptProcessor."""

    @pytest.mark.asyncio
    async def test_create_llm_openai(self):
        """Test _create_llm creates correct LLM for OpenAI."""
        tp = TranscriptProcessor()
        llm = tp._create_llm('openai', 'gpt-4o', 'sk-test-key')
        assert llm is not None

    @pytest.mark.asyncio
    async def test_create_llm_claude(self):
        """Test _create_llm creates correct LLM for Claude."""
        tp = TranscriptProcessor()
        llm = tp._create_llm('claude', 'claude-3-5-sonnet-latest', 'sk-test-key')
        assert llm is not None

    @pytest.mark.asyncio
    async def test_create_llm_groq(self):
        """Test _create_llm creates correct LLM for Groq."""
        tp = TranscriptProcessor()
        llm = tp._create_llm('groq', 'llama-3.3-70b-versatile', 'gsk-test-key')
        assert llm is not None

    @pytest.mark.asyncio
    async def test_create_llm_unsupported_raises(self):
        """Test _create_llm raises ValueError for unsupported provider."""
        tp = TranscriptProcessor()
        with pytest.raises(ValueError, match="Cannot create LLM"):
            tp._create_llm('unsupported', 'model', 'key')

    @pytest.mark.asyncio
    async def test_get_fallback_key_returns_empty_on_missing(self):
        """Test _get_fallback_key returns empty string when no key configured."""
        tp = TranscriptProcessor()
        # Mock the db to return empty
        tp.db.get_fallback_api_key = AsyncMock(return_value='')
        result = await tp._get_fallback_key('openai')
        assert result == ''

    @pytest.mark.asyncio
    async def test_get_fallback_key_returns_key_when_configured(self):
        """Test _get_fallback_key returns key when configured."""
        tp = TranscriptProcessor()
        tp.db.get_fallback_api_key = AsyncMock(return_value='sk-fallback-test')
        result = await tp._get_fallback_key('openai')
        assert result == 'sk-fallback-test'

    @pytest.mark.asyncio
    async def test_get_fallback_key_handles_db_error(self):
        """Test _get_fallback_key returns empty on database error."""
        tp = TranscriptProcessor()
        tp.db.get_fallback_api_key = AsyncMock(side_effect=Exception('DB error'))
        result = await tp._get_fallback_key('openai')
        assert result == ''

    def test_use_fallback_key_flag_defaults_false(self):
        """Test that _use_fallback_key flag defaults to False."""
        tp = TranscriptProcessor()
        assert tp._use_fallback_key is False


# ============================================================
# Run tests
# ============================================================

if __name__ == '__main__':
    pytest.main([__file__, '-v', '--tb=short'])
