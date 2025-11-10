-- Create pools table
CREATE TABLE IF NOT EXISTS pools (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    validator_hotkey VARCHAR(255) NOT NULL,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    autoscale_policy JSONB NOT NULL DEFAULT '{"min_nodes": 1, "max_nodes": 10, "scale_up_threshold": 0.8, "scale_down_threshold": 0.2, "stabilization_window_minutes": 5}',
    region VARCHAR(100),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create index on validator_hotkey for faster lookups
CREATE INDEX idx_pools_validator_hotkey ON pools(validator_hotkey);

-- Create nodes table
CREATE TABLE IF NOT EXISTS nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pool_id UUID NOT NULL REFERENCES pools(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    vmm_url VARCHAR(500) NOT NULL,
    capacity JSONB NOT NULL DEFAULT '{}',
    health JSONB NOT NULL DEFAULT '{"status": "unknown", "last_heartbeat": null, "vmm_reachable": false, "guest_agent_reachable": false, "error": null}',
    metadata JSONB DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create index on pool_id for faster lookups
CREATE INDEX idx_nodes_pool_id ON nodes(pool_id);

-- Create index on health status for monitoring
CREATE INDEX idx_nodes_health_status ON nodes((health->>'status'));

-- Create trigger to update the updated_at timestamp on pools
CREATE OR REPLACE FUNCTION update_pools_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_pools_updated_at_trigger
    BEFORE UPDATE ON pools
    FOR EACH ROW
    EXECUTE FUNCTION update_pools_updated_at();

-- Create trigger to update the updated_at timestamp on nodes  
CREATE OR REPLACE FUNCTION update_nodes_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_nodes_updated_at_trigger
    BEFORE UPDATE ON nodes
    FOR EACH ROW
    EXECUTE FUNCTION update_nodes_updated_at();
