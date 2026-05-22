-- Migration: Add Fallback API Keys to settings table
ALTER TABLE settings ADD COLUMN openaiFallbackApiKey TEXT;
ALTER TABLE settings ADD COLUMN anthropicFallbackApiKey TEXT;
ALTER TABLE settings ADD COLUMN groqFallbackApiKey TEXT;
ALTER TABLE settings ADD COLUMN openRouterFallbackApiKey TEXT;
ALTER TABLE settings ADD COLUMN geminiFallbackApiKey TEXT;
