use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    marker::PhantomData,
    ops::{Add, AddAssign, Neg},
    str::FromStr,
};

use clap::ValueEnum;
use eyre::Result;
use ulid::Ulid;

use serde::{de::Error, Deserialize, Serialize};

pub struct Id<T>(pub Ulid, PhantomData<fn() -> T>);

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl<T> Id<T> {
    pub fn generate() -> Self {
        Self::new(Ulid::new())
    }
    pub fn new(id: Ulid) -> Self {
        Self(id, PhantomData)
    }
}

impl<T> Id<Account<T>> {
    pub fn erase(self) -> Id<Account> {
        Id::new(self.0)
    }
}
impl Id<Account> {
    pub fn unerase<T>(self) -> Id<Account<T>> {
        Id::new(self.0)
    }
}

impl<T> Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use proqnt::IntoProquints;
        write!(
            f,
            "Id::<{}>::(\"{}\")",
            std::any::type_name::<T>(),
            self.0 .0.proquint_encode()
        )
    }
}

impl<T> Display for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use proqnt::IntoProquints;
        write!(f, "{}", self.0 .0.proquint_encode())
    }
}

impl<T> FromStr for Id<T> {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self> {
        use proqnt::FromProquints;
        Ok(Self::new(u128::parse_proquints(s)?.into()))
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Currency([char; 3]);
impl Currency {
    pub const EUR: Self = Self(['E', 'U', 'R']);
    pub const GBP: Self = Self(['G', 'B', 'P']);
    pub const USD: Self = Self(['U', 'S', 'D']);
}
impl Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}{}", self.0[0], self.0[1], self.0[2])
    }
}
impl FromStr for Currency {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let c: [char; 3] = s
            .chars()
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| eyre::eyre!("Requires exactly 3 upper-case chars"))?;
        if !c.iter().all(|x| x.is_ascii_uppercase()) {
            eyre::bail!("Requires exactly 3 upper-case chars");
        }
        Ok(Self(c))
    }
}

impl Serialize for Currency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

// Amount is number of smallest denomination
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount(pub i32, pub Currency);
impl Amount {
    pub fn parse_num(s: &str) -> Option<i32> {
        s.parse::<i32>().ok().map(|x| x * 100).or_else(|| {
            let (whole, cents) = s.split_once('.')?;
            if cents.len() != 2 || cents.chars().any(|c| !c.is_ascii_digit()) {
                return None;
            };
            Some(whole.parse::<i32>().ok()? * 100 + cents.parse::<i32>().ok()?)
        })
    }
}
impl Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}{} {}",
            self.0 / 100,
            if self.0 % 100 != 0 {
                format!(".{:02}", self.0 % 100)
            } else {
                "".to_owned()
            },
            self.1
        )
    }
}
impl FromStr for Amount {
    type Err = eyre::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let e = || eyre::eyre!("Amounts of currency are formatted as XXXX.XX CCC");
        let (amount, currency) = s.split_once(' ').ok_or_else(e)?;
        Ok(Self(
            Self::parse_num(amount).ok_or_else(e)?,
            currency.parse()?,
        ))
    }
}

impl Serialize for Amount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Amount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

impl Neg for Amount {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self(-self.0, self.1)
    }
}

impl Add<i32> for Amount {
    type Output = Self;
    fn add(self, rhs: i32) -> Self::Output {
        Self(self.0 + rhs, self.1)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Amounts(pub BTreeMap<Currency, Amount>);

impl AddAssign<Amount> for Amounts {
    fn add_assign(&mut self, amount: Amount) {
        let present = self.0.entry(amount.1).or_insert(Amount(0, amount.1));
        assert!(present.1 == amount.1);
        present.0 += amount.0;
    }
}

impl std::iter::Sum<Amount> for Amounts {
    fn sum<I: Iterator<Item = Amount>>(iter: I) -> Self {
        iter.fold(Self::default(), |mut acc, am| {
            acc += am;
            acc
        })
    }
}

impl Display for Amounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            itertools::intersperse(self.0.values().map(|x| x.to_string()), ", ".to_owned())
                .collect::<String>()
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physical;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Virtual;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, sqlx::Type)]
pub enum AccountType {
    Physical,
    Virtual,
}

impl FromStr for AccountType {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "physical" => Ok(Self::Physical),
            "virtual" => Ok(Self::Virtual),
            _ => Err("No such account type"),
        }
    }
}
impl Display for AccountType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                AccountType::Physical => "physical",
                AccountType::Virtual => "virtual",
            }
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account<Type = AccountType> {
    pub id: Id<Self>,
    pub name: String,
    pub notes: String,
    pub typ: Type,
    pub current: Amounts,
    pub enabled: bool,
}

impl From<Id<Account<Physical>>> for Id<Account> {
    fn from(x: Id<Account<Physical>>) -> Id<Account> {
        x.erase().unerase()
    }
}
impl From<Id<Account<Virtual>>> for Id<Account> {
    fn from(x: Id<Account<Virtual>>) -> Id<Account> {
        x.erase().unerase()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: Id<Self>,
    pub notes: String,
    pub amount: Amount,
    #[serde(flatten)]
    pub inner: TransactionInner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TransactionInner {
    Received {
        src: String,
        dst: Id<Account<Physical>>,
        dst_virt: Id<Account<Virtual>>,
    },
    Paid {
        src: Id<Account<Physical>>,
        src_virt: Id<Account<Virtual>>,
        dst: String,
    },
    MovePhys {
        src: Id<Account<Physical>>,
        dst: Id<Account<Physical>>,
    },
    MoveVirt {
        src: Id<Account<Virtual>>,
        dst: Id<Account<Virtual>>,
    },
    // Goes to the same account it came from
    Convert {
        acc: Id<Account<Physical>>,
        acc_virt: Id<Account<Virtual>>,
        new_amount: Amount,
    },
}

impl Transaction {
    pub fn results(&self) -> Vec<(Id<Account>, Amount)> {
        use TransactionInner::*;
        let &Transaction {
            amount, ref inner, ..
        } = self;
        match *inner {
            Received {
                src: _,
                dst,
                dst_virt,
            } => {
                vec![(dst.into(), amount), (dst_virt.into(), amount)]
            }
            Paid {
                src,
                src_virt,
                dst: _,
            } => vec![(src.into(), -amount), (src_virt.into(), -amount)],
            MovePhys { src, dst } => {
                vec![(src.into(), -amount), (dst.into(), amount)]
            }
            MoveVirt { src, dst } => vec![(src.into(), -amount), (dst.into(), amount)],
            Convert {
                acc,
                acc_virt,
                new_amount,
            } => vec![
                (acc.into(), -amount),
                (acc.into(), new_amount),
                (acc_virt.into(), -amount),
                (acc_virt.into(), new_amount),
            ],
        }
    }

    pub fn accounts(&self) -> [Id<Account>; 2] {
        match &self.inner {
            TransactionInner::Received {
                src: _,
                dst,
                dst_virt,
            } => [dst.erase(), dst_virt.erase()],
            TransactionInner::Paid {
                src,
                src_virt,
                dst: _,
            } => [src.erase(), src_virt.erase()],
            TransactionInner::MovePhys { src, dst } => [src.erase(), dst.erase()],
            TransactionInner::MoveVirt { src, dst } => [src.erase(), dst.erase()],
            TransactionInner::Convert {
                acc,
                acc_virt,
                new_amount: _,
            } => [acc.erase(), acc_virt.erase()],
        }
    }
}
