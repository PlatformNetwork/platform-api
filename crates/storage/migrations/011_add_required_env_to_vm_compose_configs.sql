-- Migration: Add required_env column to vm_compose_configs table
-- Created: 2025-11-17
-- Purpose: Store required environment variables for VM configurations

-- Add required_env column as JSONB array
ALTER TABLE vm_compose_configs
ADD COLUMN IF NOT EXISTS required_env JSONB DEFAULT '[]'::jsonb;

-- Update validator_vm with required environment variables
UPDATE vm_compose_configs
SET required_env = '["HOTKEY_PASSPHRASE", "DSTACK_VMM_URL", "VALIDATOR_BASE_URL"]'::jsonb
WHERE vm_type = 'validator_vm';



