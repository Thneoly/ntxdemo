use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::IpAddr;

use scheduler_core::dsl::IpPoolDef;
use scheduler_core::ip::{IpPool, ResourceType};

/// IP 池管理器
///
/// 负责管理多个 IP 池，支持：
/// - 从配置初始化 IP 池
/// - 为用户分配 IP
/// - 释放 IP
/// - 查询池状态
pub struct IpPoolManager {
    pools: HashMap<String, IpPool>,
}

impl IpPoolManager {
    /// 创建新的 IP 池管理器
    pub fn new() -> Self {
        Self {
            pools: HashMap::new(),
        }
    }

    /// 从配置定义初始化 IP 池
    ///
    /// # Arguments
    /// * `pool_defs` - IP 池配置列表
    ///
    /// # Example
    /// ```
    /// let pool_defs = vec![
    ///     IpPoolDef {
    ///         id: "pool-1".to_string(),
    ///         name: "Client Pool".to_string(),
    ///         ranges: vec!["10.0.1.0/24".to_string()],
    ///         allocation_strategy: Some("round_robin".to_string()),
    ///     }
    /// ];
    ///
    /// let mut manager = IpPoolManager::new();
    /// manager.initialize_from_config(&pool_defs)?;
    /// ```
    pub fn initialize_from_config(&mut self, pool_defs: &[IpPoolDef]) -> Result<()> {
        for def in pool_defs {
            let mut pool = IpPool::new(&def.id);

            // 添加所有 CIDR 范围
            for cidr in &def.ranges {
                pool.add_cidr_range(cidr).with_context(|| {
                    format!("Failed to add CIDR range {} to pool {}", cidr, def.id)
                })?;
            }

            let stats = pool.stats();
            println!("✓ Initialized IP pool '{}' ({})", def.id, def.name);
            println!("  - Ranges: {}", def.ranges.join(", "));
            println!("  - Total IPs: {}", stats.total);

            self.pools.insert(def.id.clone(), pool);
        }

        Ok(())
    }

    /// 为用户分配 IP
    ///
    /// # Arguments
    /// * `pool_id` - IP 池 ID
    /// * `tenant_id` - 租户 ID
    /// * `user_id` - 用户 ID
    ///
    /// # Returns
    /// 分配的 IP 地址
    pub fn allocate_ip(&mut self, pool_id: &str, tenant_id: &str, user_id: &str) -> Result<IpAddr> {
        let pool = self
            .pools
            .get_mut(pool_id)
            .with_context(|| format!("IP pool '{}' not found", pool_id))?;

        pool.allocate(
            tenant_id,
            user_id,
            ResourceType::Custom("http-client".into()),
        )
        .with_context(|| format!("Failed to allocate IP from pool '{}'", pool_id))
    }

    /// 释放 IP
    ///
    /// # Arguments
    /// * `pool_id` - IP 池 ID
    /// * `ip` - 要释放的 IP 地址
    pub fn release_ip(&mut self, pool_id: &str, ip: IpAddr) -> Result<()> {
        let pool = self
            .pools
            .get_mut(pool_id)
            .with_context(|| format!("IP pool '{}' not found", pool_id))?;

        pool.release_by_ip(&ip)
            .map(|_| ()) // 忽略返回的 IpBinding
            .with_context(|| format!("Failed to release IP {} from pool '{}'", ip, pool_id))
    }

    /// 获取池统计信息
    ///
    /// # Arguments
    /// * `pool_id` - IP 池 ID
    ///
    /// # Returns
    /// 格式化的统计信息字符串
    pub fn get_stats(&self, pool_id: &str) -> Option<String> {
        self.pools.get(pool_id).map(|pool| {
            let stats = pool.stats();
            format!(
                "Pool '{}': {} allocated, {} available (total: {})",
                pool_id, stats.allocated, stats.available, stats.total
            )
        })
    }

    /// 获取所有池的统计信息
    pub fn get_all_stats(&self) -> Vec<String> {
        self.pools
            .keys()
            .filter_map(|pool_id| self.get_stats(pool_id))
            .collect()
    }

    /// 获取池 ID 列表
    pub fn pool_ids(&self) -> Vec<&str> {
        self.pools.keys().map(|s| s.as_str()).collect()
    }

    /// 检查池是否存在
    pub fn has_pool(&self, pool_id: &str) -> bool {
        self.pools.contains_key(pool_id)
    }
}

impl Default for IpPoolManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_pool_manager_initialization() {
        let pool_defs = vec![IpPoolDef {
            id: "test-pool".to_string(),
            name: "Test Pool".to_string(),
            ranges: vec!["10.0.1.0/30".to_string()], // 2 usable IPs
            allocation_strategy: Some("round_robin".to_string()),
        }];

        let mut manager = IpPoolManager::new();
        manager
            .initialize_from_config(&pool_defs)
            .expect("should initialize");

        assert!(manager.has_pool("test-pool"));
        assert!(!manager.has_pool("missing-pool"));
    }

    #[test]
    fn test_allocate_and_release() {
        let pool_defs = vec![IpPoolDef {
            id: "test-pool".to_string(),
            name: "Test Pool".to_string(),
            ranges: vec!["10.0.1.0/30".to_string()],
            allocation_strategy: None,
        }];

        let mut manager = IpPoolManager::new();
        manager
            .initialize_from_config(&pool_defs)
            .expect("should initialize");

        // 分配第一个 IP
        let ip1 = manager
            .allocate_ip("test-pool", "tenant-a", "user-1")
            .expect("should allocate");
        assert_eq!(ip1.to_string(), "10.0.1.0");

        // 分配第二个 IP
        let ip2 = manager
            .allocate_ip("test-pool", "tenant-a", "user-2")
            .expect("should allocate");
        assert_eq!(ip2.to_string(), "10.0.1.1");

        // 分配第三个和第四个 IP
        let ip3 = manager
            .allocate_ip("test-pool", "tenant-a", "user-3")
            .expect("should allocate");
        let ip4 = manager
            .allocate_ip("test-pool", "tenant-a", "user-4")
            .expect("should allocate");

        // 池应该耗尽（/30 = 4 个 IP）
        let result = manager.allocate_ip("test-pool", "tenant-a", "user-5");
        assert!(result.is_err());

        // 释放一个 IP
        manager
            .release_ip("test-pool", ip1)
            .expect("should release");

        // 应该能再次分配
        let ip5 = manager
            .allocate_ip("test-pool", "tenant-a", "user-5")
            .expect("should allocate");
        assert_eq!(ip5.to_string(), "10.0.1.0");
    }

    #[test]
    fn test_stats() {
        let pool_defs = vec![IpPoolDef {
            id: "test-pool".to_string(),
            name: "Test Pool".to_string(),
            ranges: vec!["10.0.1.0/30".to_string()],
            allocation_strategy: None,
        }];

        let mut manager = IpPoolManager::new();
        manager
            .initialize_from_config(&pool_defs)
            .expect("should initialize");

        let stats = manager.get_stats("test-pool").unwrap();
        assert!(stats.contains("0 allocated"));
        assert!(stats.contains("4 available")); // /30 子网 = 4 个 IP

        manager
            .allocate_ip("test-pool", "tenant-a", "user-1")
            .expect("should allocate");

        let stats = manager.get_stats("test-pool").unwrap();
        assert!(stats.contains("1 allocated"));
        assert!(stats.contains("3 available")); // 4 - 1 = 3
    }
}
