/// IP Pool Management Module
///
/// This module provides IP address pool management with support for:
/// - IP range allocation and management
/// - IP allocation and release
/// - Binding IPs to resources via subinstance/subid/subtype
/// - Support for MAC address binding and other identifier types
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpPoolError {
    /// Invalid IP address format
    InvalidIpAddress(String),
    /// Invalid IP range
    InvalidRange(String),
    /// IP address not available
    IpNotAvailable(String),
    /// IP address not found in pool
    IpNotFound(String),
    /// IP address already allocated
    IpAlreadyAllocated(String),
    /// Binding not found
    BindingNotFound(String),
    /// Pool is full
    PoolFull,
    /// Invalid subnet mask
    InvalidSubnet(String),
}

impl std::fmt::Display for IpPoolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpPoolError::InvalidIpAddress(msg) => write!(f, "Invalid IP address: {}", msg),
            IpPoolError::InvalidRange(msg) => write!(f, "Invalid range: {}", msg),
            IpPoolError::IpNotAvailable(ip) => write!(f, "IP not available: {}", ip),
            IpPoolError::IpNotFound(ip) => write!(f, "IP not found: {}", ip),
            IpPoolError::IpAlreadyAllocated(ip) => write!(f, "IP already allocated: {}", ip),
            IpPoolError::BindingNotFound(msg) => write!(f, "Binding not found: {}", msg),
            IpPoolError::PoolFull => write!(f, "IP pool is full"),
            IpPoolError::InvalidSubnet(msg) => write!(f, "Invalid subnet: {}", msg),
        }
    }
}

impl std::error::Error for IpPoolError {}

/// Resource type for IP binding
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceType {
    /// MAC address (e.g., "00:11:22:33:44:55")
    Mac(String),
    /// Virtual machine identifier
    Vm(String),
    /// Container identifier
    Container(String),
    /// Pod identifier (Kubernetes)
    Pod(String),
    /// Custom resource type
    Custom(String),
}

impl ResourceType {
    pub fn as_str(&self) -> &str {
        match self {
            ResourceType::Mac(s) => s,
            ResourceType::Vm(s) => s,
            ResourceType::Container(s) => s,
            ResourceType::Pod(s) => s,
            ResourceType::Custom(s) => s,
        }
    }

    pub fn type_name(&self) -> &str {
        match self {
            ResourceType::Mac(_) => "mac",
            ResourceType::Vm(_) => "vm",
            ResourceType::Container(_) => "container",
            ResourceType::Pod(_) => "pod",
            ResourceType::Custom(_) => "custom",
        }
    }
}

/// IP binding information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpBinding {
    /// IP address
    pub ip: IpAddr,
    /// Sub-instance identifier
    pub subinstance: String,
    /// Sub-ID within the instance
    pub subid: String,
    /// Resource type and identifier
    pub subtype: ResourceType,
    /// Allocation timestamp (seconds since epoch)
    pub allocated_at: u64,
    /// Optional metadata
    pub metadata: HashMap<String, String>,
}

impl IpBinding {
    pub fn new(
        ip: IpAddr,
        subinstance: impl Into<String>,
        subid: impl Into<String>,
        subtype: ResourceType,
    ) -> Self {
        Self {
            ip,
            subinstance: subinstance.into(),
            subid: subid.into(),
            subtype,
            allocated_at: current_timestamp(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

/// IP address range
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpRange {
    /// Start IP address
    pub start: IpAddr,
    /// End IP address (inclusive)
    pub end: IpAddr,
    /// Subnet mask (CIDR notation)
    pub cidr: Option<u8>,
}

impl IpRange {
    /// Create a new IP range
    pub fn new(start: IpAddr, end: IpAddr) -> Result<Self, IpPoolError> {
        if !Self::is_valid_range(&start, &end) {
            return Err(IpPoolError::InvalidRange(format!("{} - {}", start, end)));
        }
        Ok(Self {
            start,
            end,
            cidr: None,
        })
    }

    /// Create a range from CIDR notation (e.g., "192.168.1.0/24")
    pub fn from_cidr(cidr: &str) -> Result<Self, IpPoolError> {
        let parts: Vec<&str> = cidr.split('/').collect();
        if parts.len() != 2 {
            return Err(IpPoolError::InvalidSubnet(cidr.to_string()));
        }

        let ip = IpAddr::from_str(parts[0])
            .map_err(|_| IpPoolError::InvalidIpAddress(parts[0].to_string()))?;

        let prefix_len: u8 = parts[1]
            .parse()
            .map_err(|_| IpPoolError::InvalidSubnet(cidr.to_string()))?;

        match ip {
            IpAddr::V4(ipv4) => {
                if prefix_len > 32 {
                    return Err(IpPoolError::InvalidSubnet(cidr.to_string()));
                }
                let (start, end) = Self::calculate_ipv4_range(ipv4, prefix_len);
                Ok(Self {
                    start: IpAddr::V4(start),
                    end: IpAddr::V4(end),
                    cidr: Some(prefix_len),
                })
            }
            IpAddr::V6(ipv6) => {
                if prefix_len > 128 {
                    return Err(IpPoolError::InvalidSubnet(cidr.to_string()));
                }
                let (start, end) = Self::calculate_ipv6_range(ipv6, prefix_len);
                Ok(Self {
                    start: IpAddr::V6(start),
                    end: IpAddr::V6(end),
                    cidr: Some(prefix_len),
                })
            }
        }
    }

    /// Check if an IP is within this range
    pub fn contains(&self, ip: &IpAddr) -> bool {
        match (self.start, self.end, ip) {
            (IpAddr::V4(start), IpAddr::V4(end), IpAddr::V4(ip)) => {
                u32::from(*ip) >= u32::from(start) && u32::from(*ip) <= u32::from(end)
            }
            (IpAddr::V6(start), IpAddr::V6(end), IpAddr::V6(ip)) => {
                u128::from(*ip) >= u128::from(start) && u128::from(*ip) <= u128::from(end)
            }
            _ => false,
        }
    }

    /// Get all IPs in this range (use with caution for large ranges)
    pub fn iter(&self) -> IpRangeIterator {
        IpRangeIterator::new(self.start, self.end)
    }

    fn is_valid_range(start: &IpAddr, end: &IpAddr) -> bool {
        match (start, end) {
            (IpAddr::V4(s), IpAddr::V4(e)) => u32::from(*s) <= u32::from(*e),
            (IpAddr::V6(s), IpAddr::V6(e)) => u128::from(*s) <= u128::from(*e),
            _ => false,
        }
    }

    fn calculate_ipv4_range(ip: Ipv4Addr, prefix_len: u8) -> (Ipv4Addr, Ipv4Addr) {
        let ip_u32 = u32::from(ip);
        let mask = !0u32 << (32 - prefix_len);
        let network = ip_u32 & mask;
        let broadcast = network | !mask;
        (Ipv4Addr::from(network), Ipv4Addr::from(broadcast))
    }

    fn calculate_ipv6_range(ip: Ipv6Addr, prefix_len: u8) -> (Ipv6Addr, Ipv6Addr) {
        let ip_u128 = u128::from(ip);
        let mask = !0u128 << (128 - prefix_len);
        let network = ip_u128 & mask;
        let broadcast = network | !mask;
        (Ipv6Addr::from(network), Ipv6Addr::from(broadcast))
    }
}

/// Iterator over IP addresses in a range
pub struct IpRangeIterator {
    current: Option<IpAddr>,
    end: IpAddr,
}

impl IpRangeIterator {
    fn new(start: IpAddr, end: IpAddr) -> Self {
        Self {
            current: Some(start),
            end,
        }
    }

    fn next_ipv4(ip: Ipv4Addr) -> Option<Ipv4Addr> {
        let next = u32::from(ip).checked_add(1)?;
        Some(Ipv4Addr::from(next))
    }

    fn next_ipv6(ip: Ipv6Addr) -> Option<Ipv6Addr> {
        let next = u128::from(ip).checked_add(1)?;
        Some(Ipv6Addr::from(next))
    }
}

impl Iterator for IpRangeIterator {
    type Item = IpAddr;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.current?;

        // Check if we've reached the end
        match (current, self.end) {
            (IpAddr::V4(c), IpAddr::V4(e)) if u32::from(c) > u32::from(e) => return None,
            (IpAddr::V6(c), IpAddr::V6(e)) if u128::from(c) > u128::from(e) => return None,
            _ => {}
        }

        // Prepare next value
        self.current = match current {
            IpAddr::V4(ipv4) => Self::next_ipv4(ipv4).map(IpAddr::V4),
            IpAddr::V6(ipv6) => Self::next_ipv6(ipv6).map(IpAddr::V6),
        };

        Some(current)
    }
}

/// IP Pool for managing IP address allocation
#[derive(Debug, Clone)]
pub struct IpPool {
    /// Pool name
    name: String,
    /// IP ranges in the pool
    ranges: Vec<IpRange>,
    /// Allocated IPs with their bindings
    allocated: HashMap<IpAddr, IpBinding>,
    /// Reserved IPs that cannot be allocated
    reserved: HashSet<IpAddr>,
    /// Index: subinstance -> list of IPs
    subinstance_index: HashMap<String, Vec<IpAddr>>,
    /// Index: (subinstance, subid) -> IP
    subid_index: HashMap<(String, String), IpAddr>,
    /// Index: resource type identifier -> IP
    resource_index: HashMap<String, IpAddr>,
}

impl IpPool {
    /// Create a new IP pool
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ranges: Vec::new(),
            allocated: HashMap::new(),
            reserved: HashSet::new(),
            subinstance_index: HashMap::new(),
            subid_index: HashMap::new(),
            resource_index: HashMap::new(),
        }
    }

    /// Add an IP range to the pool
    pub fn add_range(&mut self, range: IpRange) {
        self.ranges.push(range);
    }

    /// Add a range from CIDR notation
    pub fn add_cidr_range(&mut self, cidr: &str) -> Result<(), IpPoolError> {
        let range = IpRange::from_cidr(cidr)?;
        self.add_range(range);
        Ok(())
    }

    /// Reserve an IP address (prevent it from being allocated)
    pub fn reserve_ip(&mut self, ip: IpAddr) {
        self.reserved.insert(ip);
    }

    /// Unreserve an IP address
    pub fn unreserve_ip(&mut self, ip: &IpAddr) {
        self.reserved.remove(ip);
    }

    /// Allocate an IP address with binding information
    pub fn allocate(
        &mut self,
        subinstance: impl Into<String>,
        subid: impl Into<String>,
        subtype: ResourceType,
    ) -> Result<IpAddr, IpPoolError> {
        let subinstance = subinstance.into();
        let subid = subid.into();

        // Check if already allocated for this subinstance+subid
        let key = (subinstance.clone(), subid.clone());
        if let Some(existing_ip) = self.subid_index.get(&key) {
            return Ok(*existing_ip);
        }

        // Find an available IP
        let ip = self.find_available_ip().ok_or(IpPoolError::PoolFull)?;

        // Create binding
        let binding = IpBinding::new(ip, subinstance.clone(), subid.clone(), subtype.clone());

        // Update indices
        self.allocated.insert(ip, binding);
        self.subinstance_index
            .entry(subinstance.clone())
            .or_insert_with(Vec::new)
            .push(ip);
        self.subid_index.insert(key, ip);

        // Create resource identifier key
        let resource_key = format!("{}:{}", subtype.type_name(), subtype.as_str());
        self.resource_index.insert(resource_key, ip);

        Ok(ip)
    }

    /// Allocate a specific IP address
    pub fn allocate_specific(
        &mut self,
        ip: IpAddr,
        subinstance: impl Into<String>,
        subid: impl Into<String>,
        subtype: ResourceType,
    ) -> Result<(), IpPoolError> {
        let subinstance = subinstance.into();
        let subid = subid.into();

        // Check if IP is in any range
        if !self.ranges.iter().any(|r| r.contains(&ip)) {
            return Err(IpPoolError::IpNotFound(ip.to_string()));
        }

        // Check if IP is already allocated
        if self.allocated.contains_key(&ip) {
            return Err(IpPoolError::IpAlreadyAllocated(ip.to_string()));
        }

        // Check if IP is reserved
        if self.reserved.contains(&ip) {
            return Err(IpPoolError::IpNotAvailable(ip.to_string()));
        }

        // Create binding
        let binding = IpBinding::new(ip, subinstance.clone(), subid.clone(), subtype.clone());

        // Update indices
        self.allocated.insert(ip, binding);
        self.subinstance_index
            .entry(subinstance.clone())
            .or_insert_with(Vec::new)
            .push(ip);
        let key = (subinstance, subid);
        self.subid_index.insert(key, ip);

        let resource_key = format!("{}:{}", subtype.type_name(), subtype.as_str());
        self.resource_index.insert(resource_key, ip);

        Ok(())
    }

    /// Release an IP address by IP
    pub fn release_by_ip(&mut self, ip: &IpAddr) -> Result<IpBinding, IpPoolError> {
        let binding = self
            .allocated
            .remove(ip)
            .ok_or_else(|| IpPoolError::IpNotFound(ip.to_string()))?;

        // Remove from indices
        if let Some(ips) = self.subinstance_index.get_mut(&binding.subinstance) {
            ips.retain(|i| i != ip);
        }
        let key = (binding.subinstance.clone(), binding.subid.clone());
        self.subid_index.remove(&key);

        let resource_key = format!(
            "{}:{}",
            binding.subtype.type_name(),
            binding.subtype.as_str()
        );
        self.resource_index.remove(&resource_key);

        Ok(binding)
    }

    /// Release IP by subinstance and subid
    pub fn release_by_subid(
        &mut self,
        subinstance: &str,
        subid: &str,
    ) -> Result<IpBinding, IpPoolError> {
        let key = (subinstance.to_string(), subid.to_string());
        let ip =
            self.subid_index.get(&key).copied().ok_or_else(|| {
                IpPoolError::BindingNotFound(format!("{}/{}", subinstance, subid))
            })?;

        self.release_by_ip(&ip)
    }

    /// Release all IPs for a subinstance
    pub fn release_by_subinstance(&mut self, subinstance: &str) -> Vec<IpBinding> {
        let ips: Vec<IpAddr> = self
            .subinstance_index
            .get(subinstance)
            .map(|v| v.clone())
            .unwrap_or_default();

        let mut released = Vec::new();
        for ip in ips {
            if let Ok(binding) = self.release_by_ip(&ip) {
                released.push(binding);
            }
        }

        self.subinstance_index.remove(subinstance);
        released
    }

    /// Get IP binding information
    pub fn get_binding(&self, ip: &IpAddr) -> Option<&IpBinding> {
        self.allocated.get(ip)
    }

    /// Find IP by subinstance and subid
    pub fn find_by_subid(&self, subinstance: &str, subid: &str) -> Option<&IpAddr> {
        let key = (subinstance.to_string(), subid.to_string());
        self.subid_index.get(&key)
    }

    /// Find IP by resource type and identifier
    pub fn find_by_resource(&self, resource_type: &str, identifier: &str) -> Option<&IpAddr> {
        let key = format!("{}:{}", resource_type, identifier);
        self.resource_index.get(&key)
    }

    /// List all IPs for a subinstance
    pub fn list_by_subinstance(&self, subinstance: &str) -> Vec<IpAddr> {
        self.subinstance_index
            .get(subinstance)
            .map(|v| v.clone())
            .unwrap_or_default()
    }

    /// Get pool statistics
    pub fn stats(&self) -> PoolStats {
        let total = self.count_total_ips();
        let allocated = self.allocated.len();
        let reserved = self.reserved.len();
        let available = total.saturating_sub(allocated).saturating_sub(reserved);

        PoolStats {
            name: self.name.clone(),
            total,
            allocated,
            reserved,
            available,
        }
    }

    /// Get all bindings
    pub fn list_bindings(&self) -> Vec<&IpBinding> {
        self.allocated.values().collect()
    }

    fn find_available_ip(&self) -> Option<IpAddr> {
        for range in &self.ranges {
            for ip in range.iter() {
                if !self.allocated.contains_key(&ip) && !self.reserved.contains(&ip) {
                    return Some(ip);
                }
            }
        }
        None
    }

    fn count_total_ips(&self) -> usize {
        self.ranges
            .iter()
            .map(|r| {
                match (r.start, r.end) {
                    (IpAddr::V4(s), IpAddr::V4(e)) => (u32::from(e) - u32::from(s) + 1) as usize,
                    (IpAddr::V6(s), IpAddr::V6(e)) => {
                        let diff = u128::from(e).saturating_sub(u128::from(s));
                        // For IPv6, cap at usize::MAX
                        diff.min(usize::MAX as u128) as usize
                    }
                    _ => 0,
                }
            })
            .sum()
    }
}

/// Pool statistics
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolStats {
    pub name: String,
    pub total: usize,
    pub allocated: usize,
    pub reserved: usize,
    pub available: usize,
}

// Helper function to get current timestamp
fn current_timestamp() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        // In WASM, use a simple counter or get time from WASI
        0 // Placeholder
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_range_creation() {
        let start = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1));
        let end = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 254));
        let range = IpRange::new(start, end).unwrap();

        assert_eq!(range.start, start);
        assert_eq!(range.end, end);

        let test_ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100));
        assert!(range.contains(&test_ip));
    }

    #[test]
    fn test_cidr_range() {
        let range = IpRange::from_cidr("192.168.1.0/24").unwrap();

        let ip1 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0));
        let ip2 = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 255));
        let ip3 = IpAddr::V4(Ipv4Addr::new(192, 168, 2, 1));

        assert!(range.contains(&ip1));
        assert!(range.contains(&ip2));
        assert!(!range.contains(&ip3));
    }

    #[test]
    fn test_ip_pool_allocation() {
        let mut pool = IpPool::new("test-pool");
        pool.add_cidr_range("192.168.1.0/28").unwrap();

        let ip1 = pool
            .allocate(
                "instance1",
                "sub1",
                ResourceType::Mac("00:11:22:33:44:55".to_string()),
            )
            .unwrap();
        let ip2 = pool
            .allocate("instance1", "sub2", ResourceType::Vm("vm-001".to_string()))
            .unwrap();

        assert_ne!(ip1, ip2);
        assert!(pool.get_binding(&ip1).is_some());

        // Allocating same subid should return same IP
        let ip1_again = pool
            .allocate(
                "instance1",
                "sub1",
                ResourceType::Mac("00:11:22:33:44:55".to_string()),
            )
            .unwrap();
        assert_eq!(ip1, ip1_again);
    }

    #[test]
    fn test_ip_release() {
        let mut pool = IpPool::new("test-pool");
        pool.add_cidr_range("10.0.0.0/24").unwrap();

        let ip = pool
            .allocate(
                "inst1",
                "sub1",
                ResourceType::Container("container-1".to_string()),
            )
            .unwrap();
        let binding = pool.release_by_ip(&ip).unwrap();

        assert_eq!(binding.ip, ip);
        assert_eq!(binding.subinstance, "inst1");
        assert_eq!(binding.subid, "sub1");
    }

    #[test]
    fn test_find_by_resource() {
        let mut pool = IpPool::new("test-pool");
        pool.add_cidr_range("172.16.0.0/16").unwrap();

        let mac = "AA:BB:CC:DD:EE:FF";
        let _ip = pool
            .allocate("inst1", "sub1", ResourceType::Mac(mac.to_string()))
            .unwrap();

        let found_ip = pool.find_by_resource("mac", mac);
        assert!(found_ip.is_some());
    }

    #[test]
    fn test_pool_stats() {
        let mut pool = IpPool::new("test-pool");
        pool.add_cidr_range("192.168.1.0/29").unwrap(); // 8 IPs

        pool.allocate("inst1", "sub1", ResourceType::Vm("vm1".to_string()))
            .unwrap();
        pool.allocate("inst1", "sub2", ResourceType::Vm("vm2".to_string()))
            .unwrap();

        let ip = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 7));
        pool.reserve_ip(ip);

        let stats = pool.stats();
        assert_eq!(stats.total, 8);
        assert_eq!(stats.allocated, 2);
        assert_eq!(stats.reserved, 1);
        assert_eq!(stats.available, 5);
    }
}
