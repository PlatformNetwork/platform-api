-- Migration: Create vm_compose_configs table for VM compose configuration storage
-- Created: 2025-01-26
-- Purpose: Store Docker Compose files for different VM configurations

-- Table: vm_compose_configs
-- Stores Docker Compose files for VM configurations
CREATE TABLE IF NOT EXISTS vm_compose_configs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    vm_type VARCHAR(255) NOT NULL UNIQUE,
    compose_content TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX idx_vm_compose_configs_vm_type ON vm_compose_configs(vm_type);

-- Create trigger to update the updated_at timestamp
CREATE OR REPLACE FUNCTION update_vm_compose_configs_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_vm_compose_configs_updated_at_trigger
    BEFORE UPDATE ON vm_compose_configs
    FOR EACH ROW
    EXECUTE FUNCTION update_vm_compose_configs_updated_at();

-- Insert default validator_vm compose configuration
INSERT INTO vm_compose_configs (vm_type, compose_content, description)
VALUES (
    'validator_vm',
    'version: ''3.8''
services:
  validator:
    image: validator:latest
    container_name: validator_vm
    restart: unless-stopped
    ports:
      - "8080:8080"
    environment:
      - NODE_ENV=production
    volumes:
      - validator_data:/data

volumes:
  validator_data:
    driver: local',
    'Default Docker Compose configuration for validator VM'
) ON CONFLICT (vm_type) DO NOTHING;

