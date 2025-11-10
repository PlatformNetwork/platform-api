-- Migration: Create challenge_env_vars table for encrypted challenge environment variables
-- Created: 2025-01-26
-- Purpose: Store encrypted environment variables for challenges (not accessible via ORM to challenges)

-- Table: challenge_env_vars
-- Stores encrypted environment variables per challenge
CREATE TABLE IF NOT EXISTS challenge_env_vars (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    compose_hash VARCHAR(255) NOT NULL,
    key VARCHAR(255) NOT NULL,
    encrypted_value BYTEA NOT NULL,
    nonce BYTEA NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    UNIQUE(compose_hash, key)
);

CREATE INDEX idx_challenge_env_vars_compose_hash ON challenge_env_vars(compose_hash);
CREATE INDEX idx_challenge_env_vars_key ON challenge_env_vars(key);

-- Create trigger to update the updated_at timestamp
CREATE OR REPLACE FUNCTION update_challenge_env_vars_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_challenge_env_vars_updated_at_trigger
    BEFORE UPDATE ON challenge_env_vars
    FOR EACH ROW
    EXECUTE FUNCTION update_challenge_env_vars_updated_at();

