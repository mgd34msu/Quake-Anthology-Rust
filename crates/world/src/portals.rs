//! Area connectivity for numbered portals and independently counted area pairs.
use qa_core::stamps::StampSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Portal {
    pub number: u32,
    pub first: u32,
    pub second: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortalError {
    Size,
    Area,
    Number,
    Balance,
}

pub struct AreaPortals {
    count: usize,
    portals: Box<[Portal]>,
    open: Box<[bool]>,
    links: Box<[i32]>,
    components: Box<[u32]>,
    visited: StampSet,
    stack: Box<[u32]>,
}

impl AreaPortals {
    pub fn load(
        count: usize,
        numbers: usize,
        mut portals: Vec<Portal>,
    ) -> Result<Self, PortalError> {
        if count > u32::MAX as usize || numbers > u32::MAX as usize {
            return Err(PortalError::Size);
        }
        let cells = count.checked_mul(count).ok_or(PortalError::Size)?;
        for portal in &mut portals {
            if portal.first as usize >= count || portal.second as usize >= count {
                return Err(PortalError::Area);
            }
            if portal.number as usize >= numbers {
                return Err(PortalError::Number);
            }
            if portal.first > portal.second {
                std::mem::swap(&mut portal.first, &mut portal.second);
            }
        }
        portals.sort_unstable();
        portals.dedup();
        let mut links = Vec::new();
        links
            .try_reserve_exact(cells)
            .map_err(|_| PortalError::Size)?;
        links.resize(cells, 0);
        let mut result = Self {
            count,
            portals: portals.into_boxed_slice(),
            open: vec![false; numbers].into_boxed_slice(),
            links: links.into_boxed_slice(),
            components: vec![0; count].into_boxed_slice(),
            visited: StampSet::new(count),
            stack: vec![0; count].into_boxed_slice(),
        };
        result.rebuild();
        Ok(result)
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn connected(&self, first: u32, second: u32) -> Option<bool> {
        Some(self.components.get(first as usize)? == self.components.get(second as usize)?)
    }

    pub fn set(&mut self, number: u32, open: bool) -> Result<bool, PortalError> {
        let state = self.open.get(number as usize).ok_or(PortalError::Number)?;
        if *state == open {
            return Ok(false);
        }
        let start = self.portals.partition_point(|p| p.number < number);
        let end = self.portals.partition_point(|p| p.number <= number);
        let delta = if open { 1 } else { -1 };
        // Validate every affected pair before committing any count.
        for portal in &self.portals[start..end] {
            self.counts_after(portal.first, portal.second, delta)?;
        }
        for index in start..end {
            let portal = self.portals[index];
            self.change_pair(portal.first, portal.second, delta)?;
        }
        self.open[number as usize] = open;
        self.rebuild();
        Ok(true)
    }

    pub fn adjust(&mut self, first: u32, second: u32, open: bool) -> Result<(), PortalError> {
        self.change_pair(first, second, if open { 1 } else { -1 })?;
        self.rebuild();
        Ok(())
    }

    fn counts_after(
        &self,
        first: u32,
        second: u32,
        delta: i32,
    ) -> Result<(usize, usize, i32, i32), PortalError> {
        let first = first as usize;
        let second = second as usize;
        if first >= self.count || second >= self.count {
            return Err(PortalError::Area);
        }
        let a = first * self.count + second;
        let b = second * self.count + first;
        let delta = if a == b { delta * 2 } else { delta };
        let next = |old: i32| {
            old.checked_add(delta)
                .filter(|&n| n >= 0)
                .ok_or(PortalError::Balance)
        };
        Ok((a, b, next(self.links[a])?, next(self.links[b])?))
    }

    fn change_pair(&mut self, first: u32, second: u32, delta: i32) -> Result<(), PortalError> {
        let (a, b, left, right) = self.counts_after(first, second, delta)?;
        self.links[a] = left;
        self.links[b] = right;
        Ok(())
    }

    fn rebuild(&mut self) {
        self.visited.begin();
        let mut component = 0;
        for first in 0..self.count {
            if self.visited.test_and_set(first) {
                continue;
            }
            component += 1;
            self.stack[0] = first as u32;
            let mut length = 1;
            while length != 0 {
                length -= 1;
                let area = self.stack[length] as usize;
                self.components[area] = component;
                for next in 0..self.count {
                    if self.links[area * self.count + next] > 0 && !self.visited.test_and_set(next)
                    {
                        self.stack[length] = next as u32;
                        length += 1;
                    }
                }
            }
        }
    }
}
