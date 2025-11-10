-- Migration: Create platform_config table for platform-level configuration
-- Created: 2025-01-26
-- Purpose: Store encrypted CHUTES API token for platform-api (not accessible via ORM to challenges)

-- Table: platform_config
-- Stores platform-level configuration values, encrypted
CREATE TABLE IF NOT EXISTS platform_config (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key VARCHAR(255) NOT NULL UNIQUE,
    encrypted_value BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX idx_platform_config_key ON platform_config(key);

-- Create trigger to update the updated_at timestamp
CREATE OR REPLACE FUNCTION update_platform_config_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_platform_config_updated_at_trigger
    BEFORE UPDATE ON platform_config
    FOR EACH ROW
    EXECUTE FUNCTION update_platform_config_updated_at();

