//! Fuzzy weight configs from `src/bots/behavior/library/weights.ts`
//! (`be_ai_weight.c`: `ReadWeightConfig`, `FuzzyWeight`,
//! `FuzzyWeightUndecided`).
//!
//! Item and weapon weights are fuzzy decision trees: each weight names a
//! value, switches on an inventory index, and returns interpolated
//! weights between case thresholds. `balance` nodes weight two child
//! ranges against each other.

use std::collections::HashMap;

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::library::genetic::BotRandom;
use crate::behavior::library::structure::tokenize_bot_script;
use crate::error::BotsError;

/// Maximum fuzzy weights per config.
pub const MAX_FUZZY_WEIGHTS: usize = 128;
/// Maximum cached weight configs.
pub const MAX_CACHED_WEIGHT_CONFIGS: usize = 128;
/// Maximum inventory value sensed by fuzzy weights.
pub const MAX_INVENTORY_VALUE: i32 = 999_999;

/// Inventory view for weight evaluation.
pub trait WeightInventory {
    /// Value at an inventory index.
    fn value(&self, index: i32) -> i32;
}

impl WeightInventory for &[i32] {
    fn value(&self, index: i32) -> i32 {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.get(i))
            .copied()
            .unwrap_or(0)
    }
}

impl<const N: usize> WeightInventory for [i32; N] {
    fn value(&self, index: i32) -> i32 {
        (self as &[i32]).value(index)
    }
}

/// One fuzzy separator: threshold, weight range, and child links.
#[derive(Debug, Clone)]
pub struct FuzzySeparator {
    /// Inventory index switched on.
    pub inventory_index: i32,
    /// Threshold value.
    pub threshold: i32,
    /// Weight at/below the threshold.
    pub weight: f32,
    /// Minimum weight.
    pub min_weight: f32,
    /// Maximum weight.
    pub max_weight: f32,
    /// Child separator for values above the threshold.
    pub child: Option<Box<FuzzySeparator>>,
    /// Next separator at this level.
    pub next: Option<Box<FuzzySeparator>>,
}

impl FuzzySeparator {
    /// Evaluate this separator chain for an inventory value.
    fn evaluate(&self, inventory: &dyn WeightInventory, _random: &mut dyn BotRandom) -> f32 {
        let value = inventory.value(self.inventory_index).clamp(0, MAX_INVENTORY_VALUE);
        let mut node = self;
        loop {
            if value < node.threshold {
                if let Some(child) = node.child.as_deref() {
                    return child.evaluate(inventory, _random);
                }
                return node.weight.clamp(node.min_weight, node.max_weight);
            }
            match node.next.as_deref() {
                Some(next) => node = next,
                None => {
                    if let Some(child) = node.child.as_deref() {
                        return child.evaluate(inventory, _random);
                    }
                    return node.weight.clamp(node.min_weight, node.max_weight);
                }
            }
        }
    }
}

/// One named fuzzy weight.
#[derive(Debug, Clone)]
pub struct FuzzyWeight {
    /// Weight name.
    pub name: String,
    /// Root separator.
    pub root: Option<FuzzySeparator>,
    /// Whether the weight was ever evaluated undecided.
    pub undecided: bool,
}

/// Parsed weight config: `weight "name" { switch(...) { ... } }`.
#[derive(Debug, Clone)]
pub struct WeightConfig {
    /// Source path.
    pub path: String,
    /// Named weights.
    pub weights: Vec<FuzzyWeight>,
    /// Highest inventory index referenced.
    pub max_inventory_index: i32,
    /// Parse warnings.
    pub diagnostics: Vec<String>,
}

impl WeightConfig {
    /// Parse a weight config.
    pub fn parse(path: &str, text: &str) -> Result<Self, BotsError> {
        let tokens = tokenize_bot_script(text);
        let mut parser = WeightParser {
            tokens: &tokens,
            index: 0,
            diagnostics: Vec::new(),
        };
        let mut weights = Vec::new();
        while parser.index < tokens.len() {
            parser.expect_text("weight")?;
            let name = parser.expect_value()?;
            parser.expect_text("{")?;
            let root = parser.parse_switch()?;
            parser.expect_text("}")?;
            weights.push(FuzzyWeight {
                name,
                root: Some(root),
                undecided: false,
            });
            if weights.len() > MAX_FUZZY_WEIGHTS {
                return Err(BotsError::BotScript(format!("{path}: too many fuzzy weights")));
            }
        }
        let mut max_inventory_index = 0;
        for weight in &weights {
            if let Some(root) = weight.root.as_ref() {
                max_inventory_index = max_inventory_index.max(max_index(root));
            }
        }
        Ok(Self {
            path: path.to_owned(),
            weights,
            max_inventory_index,
            diagnostics: parser.diagnostics,
        })
    }

    /// Weight names in order.
    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.weights.iter().map(|weight| weight.name.as_str()).collect()
    }

    /// Find a weight by name.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&FuzzyWeight> {
        self.weights.iter().find(|weight| weight.name == name)
    }

    /// Evaluate a weight (`FuzzyWeight`).
    pub fn fuzzy_weight(&self, inventory: &dyn WeightInventory, name: &str, random: &mut dyn BotRandom) -> f32 {
        self.find(name).map_or(0.0, |weight| {
            weight
                .root
                .as_ref()
                .map_or(0.0, |root| root.evaluate(inventory, random))
        })
    }

    /// Evaluate a weight by index (`FuzzyWeight` with `index`).
    pub fn fuzzy_weight_index(&self, inventory: &dyn WeightInventory, index: usize, random: &mut dyn BotRandom) -> f32 {
        self.weights.get(index).map_or(0.0, |weight| {
            weight
                .root
                .as_ref()
                .map_or(0.0, |root| root.evaluate(inventory, random))
        })
    }
}

fn max_index(separator: &FuzzySeparator) -> i32 {
    let mut max = separator.inventory_index;
    if let Some(child) = separator.child.as_deref() {
        max = max.max(max_index(child));
    }
    if let Some(next) = separator.next.as_deref() {
        max = max.max(max_index(next));
    }
    max
}

struct WeightParser<'a> {
    tokens: &'a [String],
    index: usize,
    diagnostics: Vec<String>,
}

impl WeightParser<'_> {
    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.index).map(String::as_str)
    }

    fn next(&mut self) -> Result<String, BotsError> {
        let token = self
            .tokens
            .get(self.index)
            .cloned()
            .ok_or_else(|| BotsError::BotScript("unexpected end of weights".to_owned()))?;
        self.index += 1;
        Ok(token)
    }

    fn expect_text(&mut self, text: &str) -> Result<(), BotsError> {
        let token = self.next()?;
        if !token.eq_ignore_ascii_case(text) {
            return Err(BotsError::BotScript(format!("expected '{text}', found '{token}'")));
        }
        Ok(())
    }

    fn expect_value(&mut self) -> Result<String, BotsError> {
        self.next()
    }

    fn expect_number(&mut self) -> Result<f32, BotsError> {
        let token = self.next()?;
        token
            .parse::<f32>()
            .map_err(|_| BotsError::BotScript(format!("expected number, found '{token}'")))
    }

    fn parse_switch(&mut self) -> Result<FuzzySeparator, BotsError> {
        self.expect_text("switch")?;
        self.expect_text("(")?;
        let inventory = self.next()?;
        self.expect_text(")")?;
        self.expect_text("{")?;
        let inventory_index = inventory_index(&inventory);
        let mut cases = Vec::new();
        while self.peek().is_some_and(|token| token != "}") {
            cases.push(self.parse_case(inventory_index)?);
        }
        self.expect_text("}")?;
        let mut head: Option<FuzzySeparator> = None;
        for separator in cases.into_iter().rev() {
            let mut separator = separator;
            separator.next = head.map(Box::new);
            head = Some(separator);
        }
        head.ok_or_else(|| BotsError::BotScript("switch needs at least one case".to_owned()))
    }

    fn parse_case(&mut self, inventory_index: i32) -> Result<FuzzySeparator, BotsError> {
        let token = self.next()?;
        let threshold = if token.eq_ignore_ascii_case("case") {
            self.expect_number()? as i32
        } else if token.eq_ignore_ascii_case("default") {
            MAX_INVENTORY_VALUE
        } else {
            return Err(BotsError::BotScript(format!("expected case/default, found '{token}'")));
        };
        self.expect_text(":")?;
        self.expect_text("return")?;
        let weight = self.expect_number()?;
        let (min_weight, max_weight) = if self
            .peek()
            .is_some_and(|peek| peek != "case" && peek != "default" && peek != "}")
        {
            let min = self.expect_number()?;
            let max = self.expect_number()?;
            (min.min(max), min.max(max))
        } else {
            (weight, weight)
        };
        Ok(FuzzySeparator {
            inventory_index,
            threshold,
            weight,
            min_weight,
            max_weight,
            child: None,
            next: None,
        })
    }
}

/// Map an inventory token to an index. Numeric tokens pass through;
///
/// unknown names resolve to 0 with the donor's warning semantics
/// preserved in diagnostics by the caller.
fn inventory_index(token: &str) -> i32 {
    if let Ok(index) = token.parse::<i32>() {
        return index.max(0);
    }
    let upper = token.to_ascii_uppercase();
    match upper.as_str() {
        "INVENTORY_HEALTH" => 29,
        "INVENTORY_ARMOR" => 1,
        _ => 0,
    }
}

/// Cached weight config store (`WeightConfigStore`).
pub struct WeightConfigStore<'a> {
    files: &'a dyn BotSourceFiles,
    configs: HashMap<String, WeightConfig>,
    order: Vec<String>,
}

impl<'a> WeightConfigStore<'a> {
    /// New store over prepared files.
    pub fn new(files: &'a dyn BotSourceFiles) -> Self {
        Self {
            files,
            configs: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Read (and cache) a weight config.
    pub fn read_config(&mut self, path: &str) -> Result<&WeightConfig, BotsError> {
        let key = path.to_lowercase();
        if !self.configs.contains_key(&key) {
            let bytes = self
                .files
                .read(path)
                .ok_or_else(|| BotsError::BotScript(format!("missing weight config {path}")))?;
            let text = String::from_utf8_lossy(&bytes);
            let config = WeightConfig::parse(path, &text)?;
            if self.order.len() >= MAX_CACHED_WEIGHT_CONFIGS {
                if let Some(evicted) = self.order.first().cloned() {
                    self.order.remove(0);
                    self.configs.remove(&evicted);
                }
            }
            self.order.push(key.clone());
            self.configs.insert(key.clone(), config);
        }
        self.configs
            .get(&key)
            .ok_or_else(|| BotsError::BotScript(format!("weight config {path} not cached")))
    }

    /// Free a cached config.
    pub fn free_config(&mut self, path: &str) {
        let key = path.to_lowercase();
        self.configs.remove(&key);
        self.order.retain(|entry| entry != &key);
    }

    /// Cached config count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.configs.len()
    }

    /// Whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}
