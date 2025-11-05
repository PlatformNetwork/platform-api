use platform_api_models::*;
use uuid::Uuid;

/// Capacity requirements for a job
#[derive(Debug, Clone)]
pub struct JobRequirements {
    pub cpu: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub requires_tdx: bool,
    pub requires_gpu: bool,
    pub gpu_count: u32,
    pub region: Option<String>,
}

impl JobRequirements {
    pub fn new() -> Self {
        Self {
            cpu: 2,
            memory_gb: 4,
            disk_gb: 10,
            requires_tdx: true,
            requires_gpu: false,
            gpu_count: 0,
            region: None,
        }
    }
}

impl Default for JobRequirements {
    fn default() -> Self {
        Self::new()
    }
}

/// Score for node suitability (higher is better)
#[derive(Debug, Clone)]
pub struct NodeScore {
    pub node_id: Uuid,
    pub pool_id: Uuid,
    pub score: f64,
    pub reasons: Vec<String>,
}

/// Binpacking algorithm for node selection
pub struct CapacityMatcher;

impl CapacityMatcher {
    /// Find best node for job requirements using binpacking
    pub fn find_best_node(
        nodes: &[Node],
        requirements: &JobRequirements,
    ) -> Option<NodeScore> {
        let mut candidates: Vec<NodeScore> = nodes
            .iter()
            .filter_map(|node| {
                let score = Self::score_node(node, requirements)?;
                Some(NodeScore {
                    node_id: node.id,
                    pool_id: node.pool_id,
                    score,
                    reasons: Self::get_match_reasons(node, requirements),
                })
            })
            .collect();

        // Sort by score descending
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        
        candidates.into_iter().next()
    }

    /// Score a node's fitness for requirements (0.0 to 1.0)
    fn score_node(node: &Node, requirements: &JobRequirements) -> Option<f64> {
        // Check mandatory requirements
        if requirements.requires_tdx && !node.capacity.has_tdx {
            return None;
        }
        
        if requirements.requires_gpu && node.capacity.gpu_count < requirements.gpu_count {
            return None;
        }
        
        if let Some(ref region) = requirements.region {
            if node.capacity.region != *region {
                return None;
            }
        }
        
        // Check capacity availability
        if node.capacity.available_cpu < requirements.cpu {
            return None;
        }
        
        if node.capacity.available_memory_gb < requirements.memory_gb {
            return None;
        }
        
        if node.capacity.available_disk_gb < requirements.disk_gb {
            return None;
        }
        
        // Check health
        if !matches!(node.health.status, HealthStatus::Healthy) {
            return None;
        }
        
        // Calculate fitness score
        let cpu_score = node.capacity.available_cpu as f64 / node.capacity.total_cpu as f64;
        let memory_score = node.capacity.available_memory_gb as f64 / node.capacity.total_memory_gb as f64;
        let disk_score = node.capacity.available_disk_gb as f64 / node.capacity.total_disk_gb as f64;
        
        // Weighted average (CPU most important)
        let score = (cpu_score * 0.5 + memory_score * 0.3 + disk_score * 0.2);
        
        Some(score)
    }

    /// Get reasons why a node matches
    fn get_match_reasons(node: &Node, requirements: &JobRequirements) -> Vec<String> {
        let mut reasons = Vec::new();
        
        if node.capacity.has_tdx {
            reasons.push("TDX enabled".to_string());
        }
        
        if node.capacity.gpu_count > 0 {
            reasons.push(format!("GPU available ({})", node.capacity.gpu_count));
        }
        
        if node.capacity.available_cpu >= requirements.cpu {
            reasons.push(format!("CPU capacity: {} available", node.capacity.available_cpu));
        }
        
        if node.capacity.available_memory_gb >= requirements.memory_gb {
            reasons.push(format!("Memory capacity: {}GB available", node.capacity.available_memory_gb));
        }
        
        reasons
    }

    /// Check if a pool has sufficient capacity
    pub fn check_pool_capacity(
        _pool: &platform_api_models::Pool,
        capacity: &PoolCapacitySummary,
        requirements: &JobRequirements,
    ) -> bool {
        // Check mandatory requirements
        if requirements.requires_tdx && !capacity.has_tdx {
            return false;
        }
        
        if requirements.requires_gpu && capacity.gpu_count < requirements.gpu_count {
            return false;
        }
        
        // Check available capacity
        capacity.available_cpu >= requirements.cpu
            && capacity.available_memory_gb >= requirements.memory_gb
            && capacity.healthy_nodes > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_scoring() {
        let mut node = Node {
            id: Uuid::new_v4(),
            pool_id: Uuid::new_v4(),
            name: "test-node".to_string(),
            vmm_url: "http://localhost:8080".to_string(),
            capacity: NodeCapacity {
                total_cpu: 8,
                total_memory_gb: 32,
                total_disk_gb: 100,
                available_cpu: 4,
                available_memory_gb: 16,
                available_disk_gb: 50,
                has_tdx: true,
                has_gpu: false,
                gpu_count: 0,
                gpu_model: None,
                region: "us-east-1".to_string(),
            },
            health: NodeHealth::default(),
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        
        node.health.status = HealthStatus::Healthy;
        
        let requirements = JobRequirements {
            cpu: 2,
            memory_gb: 4,
            disk_gb: 10,
            requires_tdx: true,
            requires_gpu: false,
            gpu_count: 0,
            region: None,
        };
        
        let score = CapacityMatcher::score_node(&node, &requirements);
        assert!(score.is_some());
        assert!(score.unwrap() > 0.0);
    }
}

