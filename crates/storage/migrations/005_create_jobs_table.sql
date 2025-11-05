-- Migration: Create jobs table for scheduler
-- Created: 2025-11-04

-- Table: jobs
-- Stores job metadata for evaluation tasks
CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    challenge_id UUID NOT NULL,
    validator_hotkey VARCHAR(255),
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    priority VARCHAR(50) NOT NULL DEFAULT 'normal',
    runtime VARCHAR(50) NOT NULL DEFAULT 'docker',
    payload JSONB NOT NULL,
    result JSONB,
    error_message TEXT,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    claimed_at TIMESTAMP WITH TIME ZONE,
    started_at TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE,
    timeout_at TIMESTAMP WITH TIME ZONE,
    retry_count INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 3
);

CREATE INDEX idx_jobs_challenge_id ON jobs(challenge_id);
CREATE INDEX idx_jobs_status ON jobs(status);
CREATE INDEX idx_jobs_validator_hotkey ON jobs(validator_hotkey);
CREATE INDEX idx_jobs_created_at ON jobs(created_at);
CREATE INDEX idx_jobs_pending ON jobs(status, created_at) WHERE status = 'pending';

