use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, PartialEq)]
pub enum Col {
    Ts,
    Open,
    High,
    Low,
    Close,
    Volume,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Cmp(Col, Op, f32),
    And(Box<Predicate>, Box<Predicate>),
    Or(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

impl Hash for Col {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
    }
}

impl Hash for Op {
    fn hash<H: Hasher>(&self, state: &mut H) {
        core::mem::discriminant(self).hash(state);
    }
}

impl Hash for Predicate {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Predicate::Cmp(col, op, val) => {
                0u8.hash(state);
                col.hash(state);
                op.hash(state);
                val.to_bits().hash(state);
            }
            Predicate::And(a, b) => {
                1u8.hash(state);
                a.hash(state);
                b.hash(state);
            }
            Predicate::Or(a, b) => {
                2u8.hash(state);
                a.hash(state);
                b.hash(state);
            }
            Predicate::Not(p) => {
                3u8.hash(state);
                p.hash(state);
            }
        }
    }
}

impl Eq for Predicate {}
