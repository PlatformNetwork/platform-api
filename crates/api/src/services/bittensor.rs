use anyhow::{anyhow, Result};
use bittensor_rs::chain::BittensorClient;
use bittensor_rs::queries::subnets;
use platform_api_models::{
    ChallengeEmissionBreakdown, ChallengeEmissions, MechanismEmissionBreakdown, MechanismEmissions,
    SubnetEmissions,
};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Cache entry for emission data
struct EmissionCacheEntry<T> {
    data: T,
    expires_at: SystemTime,
}

/// Bittensor service for querying blockchain emission data
pub struct BittensorService {
    client: Arc<BittensorClient>,
    netuid: u16,
    cache_ttl: Duration,
    subnet_cache: Arc<RwLock<Option<EmissionCacheEntry<SubnetEmissions>>>>,
    mechanism_cache:
        Arc<RwLock<std::collections::HashMap<u8, EmissionCacheEntry<MechanismEmissions>>>>,
    challenge_cache:
        Arc<RwLock<std::collections::HashMap<Uuid, EmissionCacheEntry<ChallengeEmissions>>>>,
}

impl BittensorService {
    /// Create a new BittensorService
    pub async fn new(netuid: Option<u16>, endpoint: Option<String>) -> Result<Self> {
        let netuid = netuid.unwrap_or_else(|| {
            std::env::var("BT_NETUID")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100)
        });

        let client = if let Some(endpoint) = endpoint {
            BittensorClient::new(&endpoint).await.map_err(|e| {
                anyhow!(
                    "Failed to create Bittensor client with endpoint {}: {}",
                    endpoint,
                    e
                )
            })?
        } else {
            BittensorClient::with_default()
                .await
                .map_err(|e| anyhow!("Failed to create Bittensor client: {}", e))?
        };

        let cache_ttl = Duration::from_secs(
            std::env::var("EMISSION_CACHE_TTL")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(300), // Default 5 minutes
        );

        info!(
            netuid = netuid,
            cache_ttl_secs = cache_ttl.as_secs(),
            "Initialized BittensorService"
        );

        Ok(Self {
            client: Arc::new(client),
            netuid,
            cache_ttl,
            subnet_cache: Arc::new(RwLock::new(None)),
            mechanism_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            challenge_cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Check if cache entry is still valid
    fn is_cache_valid<T>(entry: &Option<EmissionCacheEntry<T>>) -> bool {
        if let Some(entry) = entry {
            entry.expires_at > SystemTime::now()
        } else {
            false
        }
    }

    /// Calculate total subnet emissions per day
    pub async fn calculate_subnet_emissions(
        &self,
        challenge_registry: &std::collections::HashMap<String, platform_api_models::ChallengeSpec>,
    ) -> Result<SubnetEmissions> {
        // Check cache first
        {
            let cache = self.subnet_cache.read().await;
            if let Some(entry) = cache.as_ref() {
                if entry.expires_at > SystemTime::now() {
                    return Ok(entry.data.clone());
                }
            }
        }

        debug!(
            netuid = self.netuid,
            "Calculating subnet emissions from chain"
        );

        // Get block emission (in RAO)
        // Subtensor emission schedule:
        // - 1 block every 12 seconds
        // - 1 TAO per block = 1_000_000_000 RAO per block
        // - 7200 blocks per day = 7200 TAO per day
        // BlockEmission is a StorageValue with ValueQuery, so it should return the default (1 TAO = 1e9 RAO)
        // if not set. However, if storage() returns None, the entry doesn't exist in runtime metadata.
        let block_emission_rao = match subnets::block_emission(&self.client).await {
            Ok(Some(emission)) => {
                debug!(
                    emission_rao = emission,
                    emission_tao = emission as f64 / 1e9,
                    "Retrieved block emission from chain"
                );
                emission
            }
            Ok(None) => {
                warn!("Block emission storage entry 'BlockEmission' returned None");
                warn!("  This means the storage entry doesn't exist in the Subtensor runtime metadata.");
                warn!("  According to Subtensor source, DefaultBlockEmission = 1_000_000_000 RAO (1 TAO per block)");
                warn!("  Subtensor emission: 1 TAO per block, 1 block every 12s, 7200 TAO per day");
                warn!("  Using default value: 1 TAO per block (1_000_000_000 RAO)");
                1_000_000_000u64 // Default from Subtensor: 1 TAO per block
            }
            Err(e) => {
                warn!(error = %e, "Failed to query block emission storage, using default value");
                warn!("  Using Subtensor default: 1 TAO per block (1_000_000_000 RAO)");
                warn!("  Subtensor emission: 1 TAO per block, 1 block every 12s, 7200 TAO per day");
                1_000_000_000u64
            }
        };

        // Get tempo (blocks per epoch)
        let tempo = match subnets::tempo(&self.client, self.netuid).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                warn!(
                    netuid = self.netuid,
                    "Tempo not available from chain, using default value"
                );
                100u64 // Default tempo if not available
            }
            Err(e) => {
                warn!(netuid = self.netuid, error = %e, "Failed to query tempo, using default value");
                100u64
            }
        };

        // Get subnet emission percentage
        let subnet_emission_percent = subnets::subnet_emission_percent(&self.client, self.netuid)
            .await?
            .unwrap_or(0.0);

        // Calculate subnet block emission (RAO per block)
        let subnet_block_emission_rao =
            (block_emission_rao as f64 * subnet_emission_percent) as u64;

        // Calculate daily emissions: (blocks_per_day) * (emission_per_block)
        // Blocks per day = (86400 seconds / block_time_seconds)
        // Subtensor: 1 block every 12 seconds = 7200 blocks per day
        // Emission: 1 TAO per block = 7200 TAO per day
        let blocks_per_day = 86400.0 / 12.0; // 7200 blocks per day (1 block every 12 seconds)
        let daily_emissions_rao = subnet_block_emission_rao as f64 * blocks_per_day;
        let daily_emissions_tao = daily_emissions_rao / 1e9;

        // Get mechanism count
        // Default to 1 if not available (at least one mechanism exists)
        let mechanism_count = subnets::mechanism_count(&self.client, self.netuid)
            .await?
            .unwrap_or(1u64);

        // Get mechanism emission split percentages
        // Note: The git dependency version returns Option<u64>, so we use even distribution
        // In the future when bittensor-rs is updated, this can use Vec<u16> split
        let mechanism_percentages = vec![1.0 / mechanism_count as f64; mechanism_count as usize];

        // Build mechanism breakdowns
        let mut mechanisms = Vec::new();
        for mechanism_id in 0..mechanism_count {
            let mechanism_id_u8 = mechanism_id as u8;
            let emission_percentage = mechanism_percentages
                .get(mechanism_id as usize)
                .copied()
                .unwrap_or(1.0 / mechanism_count as f64);

            let mechanism_daily_emissions_tao = daily_emissions_tao * emission_percentage;

            // Get challenges for this mechanism from registry
            let mechanism_challenges: Vec<_> = challenge_registry
                .values()
                .filter(|challenge| challenge.mechanism_id == mechanism_id_u8)
                .collect();

            // Calculate total emission share for normalization
            let total_emission_share: f64 = mechanism_challenges
                .iter()
                .map(|challenge| challenge.emission_share)
                .sum();

            // If no emission shares are set, distribute evenly
            let normalized_shares = if total_emission_share == 0.0 {
                let count = mechanism_challenges.len() as f64;
                mechanism_challenges
                    .iter()
                    .map(|_| 1.0 / count.max(1.0))
                    .collect::<Vec<_>>()
            } else {
                mechanism_challenges
                    .iter()
                    .map(|challenge| challenge.emission_share / total_emission_share)
                    .collect::<Vec<_>>()
            };

            let challenges: Vec<ChallengeEmissionBreakdown> = mechanism_challenges
                .iter()
                .zip(normalized_shares.iter())
                .map(|(challenge, &share)| {
                    let challenge_daily_emissions_tao = mechanism_daily_emissions_tao * share;

                    ChallengeEmissionBreakdown {
                        challenge_id: challenge.id,
                        challenge_name: challenge.name.clone(),
                        emission_share: share,
                        daily_emissions_tao: challenge_daily_emissions_tao,
                    }
                })
                .collect();

            mechanisms.push(MechanismEmissionBreakdown {
                mechanism_id: mechanism_id_u8,
                emission_percentage,
                daily_emissions_tao: mechanism_daily_emissions_tao,
                challenges,
            });
        }

        let result = SubnetEmissions {
            netuid: self.netuid,
            total_daily_emissions_tao: daily_emissions_tao,
            block_emission_rao: subnet_block_emission_rao,
            tempo,
            subnet_emission_percent,
            mechanisms,
        };

        // Update cache
        {
            let mut cache = self.subnet_cache.write().await;
            *cache = Some(EmissionCacheEntry {
                data: result.clone(),
                expires_at: SystemTime::now() + self.cache_ttl,
            });
        }

        Ok(result)
    }

    /// Calculate emissions for a specific mechanism
    pub async fn calculate_mechanism_emissions(
        &self,
        mechanism_id: u8,
        challenge_registry: &std::collections::HashMap<String, platform_api_models::ChallengeSpec>,
    ) -> Result<MechanismEmissions> {
        // Check cache first
        {
            let cache = self.mechanism_cache.read().await;
            if let Some(entry) = cache.get(&mechanism_id) {
                if entry.expires_at > SystemTime::now() {
                    return Ok(entry.data.clone());
                }
            }
        }

        // Get subnet emissions first
        let subnet_emissions = self.calculate_subnet_emissions(challenge_registry).await?;

        // Find the mechanism
        let mechanism = subnet_emissions
            .mechanisms
            .iter()
            .find(|m| m.mechanism_id == mechanism_id)
            .ok_or_else(|| anyhow!("Mechanism {} not found", mechanism_id))?;

        let result = MechanismEmissions {
            netuid: self.netuid,
            mechanism_id,
            emission_percentage: mechanism.emission_percentage,
            daily_emissions_tao: mechanism.daily_emissions_tao,
            challenges: mechanism.challenges.clone(),
        };

        // Update cache
        {
            let mut cache = self.mechanism_cache.write().await;
            cache.insert(
                mechanism_id,
                EmissionCacheEntry {
                    data: result.clone(),
                    expires_at: SystemTime::now() + self.cache_ttl,
                },
            );
        }

        Ok(result)
    }

    /// Calculate emissions for a specific challenge
    pub async fn calculate_challenge_emissions(
        &self,
        challenge_id: Uuid,
        challenge_registry: &std::collections::HashMap<String, platform_api_models::ChallengeSpec>,
    ) -> Result<ChallengeEmissions> {
        // Check cache first
        {
            let cache = self.challenge_cache.read().await;
            if let Some(entry) = cache.get(&challenge_id) {
                if entry.expires_at > SystemTime::now() {
                    return Ok(entry.data.clone());
                }
            }
        }

        // Find challenge in registry
        let challenge = challenge_registry
            .values()
            .find(|c| c.id == challenge_id)
            .ok_or_else(|| anyhow!("Challenge {} not found", challenge_id))?;

        // Get mechanism emissions
        let mechanism_emissions = self
            .calculate_mechanism_emissions(challenge.mechanism_id, challenge_registry)
            .await?;

        // Find challenge in mechanism
        let challenge_breakdown = mechanism_emissions
            .challenges
            .iter()
            .find(|c| c.challenge_id == challenge_id)
            .ok_or_else(|| anyhow!("Challenge {} not found in mechanism", challenge_id))?;

        let result = ChallengeEmissions {
            netuid: self.netuid,
            challenge_id,
            challenge_name: challenge_breakdown.challenge_name.clone(),
            mechanism_id: challenge.mechanism_id,
            mechanism_emission_percentage: mechanism_emissions.emission_percentage,
            challenge_emission_share: challenge_breakdown.emission_share,
            daily_emissions_tao: challenge_breakdown.daily_emissions_tao,
        };

        // Update cache
        {
            let mut cache = self.challenge_cache.write().await;
            cache.insert(
                challenge_id,
                EmissionCacheEntry {
                    data: result.clone(),
                    expires_at: SystemTime::now() + self.cache_ttl,
                },
            );
        }

        Ok(result)
    }
}

impl Clone for BittensorService {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            netuid: self.netuid,
            cache_ttl: self.cache_ttl,
            subnet_cache: self.subnet_cache.clone(),
            mechanism_cache: self.mechanism_cache.clone(),
            challenge_cache: self.challenge_cache.clone(),
        }
    }
}
