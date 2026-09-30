//! Case composition products (donor `tools/verify/product.ts`).
//!
//! Validates composition domains, counts and enumerates their case products,
//! and assigns cases to deterministic shards.

use std::collections::HashMap;

use crate::error::ToolsError;
use crate::json::Json;
use crate::verify::hash::hash_json;
use crate::verify::schema::{CompositionDomain, ExpectedCase, InputRequirement};

/// Arbitrary-precision case count (donor `bigint`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseCount(Vec<u32>);

impl CaseCount {
    /// Zero.
    #[must_use]
    pub fn zero() -> Self {
        Self(vec![0])
    }

    /// Build from a `u64`.
    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        let mut limbs = vec![(value & 0xffff_ffff) as u32, (value >> 32) as u32];
        while limbs.len() > 1 && limbs.last() == Some(&0) {
            limbs.pop();
        }
        Self(limbs)
    }

    /// Parse a decimal count (`--max-cases` values).
    pub fn parse_decimal(text: &str) -> Result<Self, ToolsError> {
        if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(ToolsError::invalid(format!("Invalid case count: {text}")));
        }
        let mut count = Self::from_u64(0);
        for byte in text.bytes() {
            count.mul_small(10);
            count.add_small(u64::from(byte - b'0'));
        }
        Ok(count)
    }

    fn normalize(&mut self) {
        while self.0.len() > 1 && self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn add_small(&mut self, mut value: u64) {
        let mut index = 0;
        while value > 0 {
            if index >= self.0.len() {
                self.0.push(0);
            }
            let sum = u64::from(self.0[index]) + (value & 0xffff_ffff);
            self.0[index] = (sum & 0xffff_ffff) as u32;
            value = (value >> 32) + (sum >> 32);
            index += 1;
        }
        self.normalize();
    }

    fn mul_small(&mut self, factor: u64) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let product = u64::from(*limb) * factor + carry;
            *limb = (product & 0xffff_ffff) as u32;
            carry = product >> 32;
        }
        while carry > 0 {
            self.0.push((carry & 0xffff_ffff) as u32);
            carry >>= 32;
        }
        self.normalize();
    }

    fn div_mod_small(&self, divisor: u64) -> (Self, u64) {
        debug_assert!(divisor > 0);
        let mut limbs = vec![0u32; self.0.len()];
        let mut remainder = 0u64;
        for (index, limb) in self.0.iter().enumerate().rev() {
            let current = (remainder << 32) | u64::from(*limb);
            limbs[index] = (current / divisor) as u32;
            remainder = current % divisor;
        }
        let mut result = Self(limbs);
        result.normalize();
        (result, remainder)
    }

    /// Whether the count is zero.
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0.iter().all(|limb| *limb == 0)
    }

    /// Render as decimal.
    #[must_use]
    pub fn to_decimal(&self) -> String {
        if self.is_zero() {
            return "0".to_owned();
        }
        let mut digits = Vec::new();
        let mut current = self.clone();
        while !current.is_zero() {
            let (next, remainder) = current.div_mod_small(10);
            digits.push(b'0' + remainder as u8);
            current = next;
        }
        digits.reverse();
        String::from_utf8(digits).unwrap_or_else(|_| "0".to_owned())
    }
}

impl PartialOrd for CaseCount {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CaseCount {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&other.0.len())
            .then_with(|| self.0.iter().rev().cmp(other.0.iter().rev()))
    }
}

impl std::fmt::Display for CaseCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_decimal())
    }
}

fn unique(values: &[String], description: &str) -> Result<(), ToolsError> {
    let mut seen = std::collections::HashSet::new();
    for value in values {
        if value.is_empty() || !seen.insert(value) {
            return Err(ToolsError::invalid(format!(
                "Empty or duplicate {description}: {value}"
            )));
        }
    }
    Ok(())
}

/// Validate a composition domain.
pub fn validate_domain(domain: &CompositionDomain) -> Result<(), ToolsError> {
    if domain.id.is_empty() {
        return Err(ToolsError::invalid("Invalid composition domain identity"));
    }
    if domain.axes.is_empty() || domain.suites.is_empty() {
        return Err(ToolsError::invalid(
            "A composition domain needs axes and required suites",
        ));
    }
    unique(
        &domain.axes.iter().map(|axis| axis.id.clone()).collect::<Vec<_>>(),
        "axis ID",
    )?;
    unique(
        &domain.suites.iter().map(|suite| suite.id.clone()).collect::<Vec<_>>(),
        "suite ID",
    )?;
    for axis in &domain.axes {
        if axis.values.is_empty() {
            return Err(ToolsError::invalid(format!(
                "Empty domain axis {}: retain a named missing-input value",
                axis.id
            )));
        }
        unique(
            &axis.values.iter().map(|value| value.id.clone()).collect::<Vec<_>>(),
            &format!("value ID in {}", axis.id),
        )?;
    }
    for suite in &domain.suites {
        if suite.contracts.is_empty() {
            return Err(ToolsError::invalid(format!(
                "Suite {} has no expected contract",
                suite.id
            )));
        }
        if suite.profiles.is_empty() {
            return Err(ToolsError::invalid(format!(
                "Suite {} has no verification profile",
                suite.id
            )));
        }
        unique(
            &suite
                .contracts
                .iter()
                .map(|contract| contract.id.clone())
                .collect::<Vec<_>>(),
            &format!("contract ID in {}", suite.id),
        )?;
        for contract in &suite.contracts {
            if contract.minimum_assertions <= 0 {
                return Err(ToolsError::invalid(format!(
                    "Contract {} must require at least one assertion",
                    contract.id
                )));
            }
        }
    }
    Ok(())
}

/// Count the cases in a composition product.
pub fn composition_size(domain: &CompositionDomain) -> Result<CaseCount, ToolsError> {
    validate_domain(domain)?;
    let mut total = CaseCount::from_u64(domain.suites.len() as u64);
    for axis in &domain.axes {
        total.mul_small(axis.values.len() as u64);
    }
    Ok(total)
}

fn combine_requirements(requirements: &[InputRequirement]) -> Result<Vec<InputRequirement>, ToolsError> {
    let mut combined: HashMap<&str, &InputRequirement> = HashMap::new();
    for requirement in requirements {
        if let Some(previous) = combined.get(requirement.id.as_str()) {
            if hash_json(&previous.to_json())? != hash_json(&requirement.to_json())? {
                return Err(ToolsError::invalid(format!(
                    "Conflicting input requirement {}",
                    requirement.id
                )));
            }
        }
        combined.insert(requirement.id.as_str(), requirement);
    }
    let mut merged: Vec<InputRequirement> = combined.into_values().cloned().collect();
    merged.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(merged)
}

/// `encodeURIComponent` for suite ids.
#[must_use]
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

struct AxisPlan<'a> {
    id: &'a str,
    values: Vec<&'a crate::verify::schema::AxisValue>,
}

/// Lazy composition iterator (donor `generateComposition`).
pub struct Composition<'a> {
    domain: &'a CompositionDomain,
    axes: Vec<AxisPlan<'a>>,
    suites: Vec<&'a crate::verify::schema::CompositionSuite>,
    positions: Vec<usize>,
    suite_index: usize,
    started: bool,
    finished: bool,
}

impl<'a> Composition<'a> {
    /// Borrow a validated domain for iteration.
    pub fn new(domain: &'a CompositionDomain) -> Result<Self, ToolsError> {
        validate_domain(domain)?;
        let mut axes: Vec<AxisPlan> = domain
            .axes
            .iter()
            .map(|axis| {
                let mut values: Vec<&crate::verify::schema::AxisValue> = axis.values.iter().collect();
                values.sort_by(|left, right| left.id.cmp(&right.id));
                AxisPlan {
                    id: axis.id.as_str(),
                    values,
                }
            })
            .collect();
        axes.sort_by(|left, right| left.id.cmp(right.id));
        let mut suites: Vec<&crate::verify::schema::CompositionSuite> = domain.suites.iter().collect();
        suites.sort_by(|left, right| left.id.cmp(&right.id));
        let positions = vec![0; axes.len()];
        Ok(Self {
            domain,
            axes,
            suites,
            positions,
            suite_index: 0,
            started: false,
            finished: false,
        })
    }

    fn current(&self) -> Result<ExpectedCase, ToolsError> {
        let mut configuration: Vec<(String, String)> = Vec::with_capacity(self.axes.len());
        let mut requirements: Vec<InputRequirement> = self.domain.requirements.to_vec();
        for (axis, position) in self.axes.iter().zip(self.positions.iter()) {
            let value = axis.values[*position];
            configuration.push((axis.id.to_owned(), value.id.clone()));
            requirements.extend(value.requirements.iter().cloned());
        }
        let config_json = Json::object(
            configuration
                .iter()
                .map(|(key, value)| (key.clone(), Json::string(value)))
                .collect(),
        );
        let configuration_id = format!("{}/{}", self.domain.id, hash_json(&config_json)?);
        let suite = self.suites[self.suite_index];
        Ok(ExpectedCase {
            id: format!("{}/{}", configuration_id, encode_uri_component(&suite.id)),
            configuration_id,
            suite_id: suite.id.clone(),
            evidence_kind: suite.evidence_kind,
            configuration,
            profiles: suite.profiles.clone(),
            requirements: combine_requirements(&requirements)?,
            contracts: suite.contracts.clone(),
            seed: self.domain.seed,
            clock_schedule_sha256: self.domain.clock_schedule_sha256.clone(),
            network_schedule_sha256: self.domain.network_schedule_sha256.clone(),
            source_paths: self.domain.source_paths.clone(),
            command: suite.command.clone(),
        })
    }
}

fn carry_positions(positions: &mut [usize], axes: &[AxisPlan], finished: &mut bool) {
    let mut index = positions.len();
    while index > 0 {
        index -= 1;
        if positions[index] + 1 < axes[index].values.len() {
            positions[index] += 1;
            return;
        }
        positions[index] = 0;
    }
    *finished = true;
}

impl Iterator for Composition<'_> {
    type Item = Result<ExpectedCase, ToolsError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        if self.started {
            // Move to the next suite, or to the next axis combination.
            self.suite_index += 1;
            if self.suite_index >= self.suites.len() {
                self.suite_index = 0;
                carry_positions(&mut self.positions, &self.axes, &mut self.finished);
                if self.finished {
                    return None;
                }
            }
        } else {
            self.started = true;
        }
        Some(self.current())
    }
}

/// Enumerate a composition product.
pub fn generate_composition(domain: &CompositionDomain) -> Result<Composition<'_>, ToolsError> {
    Composition::new(domain)
}

/// A deterministic case shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard {
    /// Shard index.
    pub index: i64,
    /// Shard count.
    pub count: i64,
}

/// Parse `INDEX/COUNT`.
pub fn parse_shard(value: &str) -> Result<Shard, ToolsError> {
    let (index_text, count_text) = value
        .split_once('/')
        .ok_or_else(|| ToolsError::invalid("Shard must be INDEX/COUNT"))?;
    if index_text.is_empty()
        || count_text.is_empty()
        || !index_text.bytes().all(|byte| byte.is_ascii_digit())
        || !count_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ToolsError::invalid("Shard must be INDEX/COUNT"));
    }
    let index: i64 = index_text
        .parse()
        .map_err(|_| ToolsError::invalid("Shard requires 0 <= INDEX < COUNT"))?;
    let count: i64 = count_text
        .parse()
        .map_err(|_| ToolsError::invalid("Shard requires 0 <= INDEX < COUNT"))?;
    if count < 1 || index < 0 || index >= count {
        return Err(ToolsError::invalid("Shard requires 0 <= INDEX < COUNT"));
    }
    Ok(Shard { index, count })
}

/// Whether a case belongs to a shard (hex digest modulo count).
pub fn belongs_to_shard(case_id: &str, seed: i64, shard: Shard) -> Result<bool, ToolsError> {
    if shard.index < 0 || shard.count < 1 || shard.index >= shard.count {
        return Err(ToolsError::invalid("Invalid shard"));
    }
    let digest = hash_json(&Json::object(vec![
        ("caseId".to_owned(), Json::string(case_id)),
        ("seed".to_owned(), Json::int(seed)),
    ]))?;
    let mut remainder: i64 = 0;
    for digit in digest.bytes() {
        let value = (digit as char).to_digit(16).unwrap_or(0) as i64;
        remainder = (remainder * 16 + value) % shard.count;
    }
    Ok(remainder == shard.index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_count_arithmetic() {
        let mut count = CaseCount::from_u64(123);
        count.mul_small(456);
        assert_eq!(count.to_decimal(), "56088");
        assert_eq!(
            CaseCount::parse_decimal("100000000000000000000000000")
                .unwrap()
                .to_decimal(),
            "100000000000000000000000000"
        );
        assert!(CaseCount::parse_decimal("12a").is_err());
        assert!(CaseCount::from_u64(3) < CaseCount::from_u64(10));
    }

    #[test]
    fn shard_parsing() {
        assert_eq!(parse_shard("0/1").unwrap(), Shard { index: 0, count: 1 });
        assert!(parse_shard("1/1").is_err());
        assert!(parse_shard("a/b").is_err());
        assert!(parse_shard("0/0").is_err());
    }

    #[test]
    fn uri_component_encoding() {
        assert_eq!(encode_uri_component("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(encode_uri_component("suite-1_name.v2!~*'()"), "suite-1_name.v2!~*'()");
    }
}
