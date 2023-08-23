use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    marker::PhantomData,
    str::FromStr, ops::{Add, Neg},
};

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
    pub fn erase(self) -> Id<()> {
        Id::new(self.0)
    }
}
impl Id<()> {
    pub fn unerase<T>(self) -> Id<T> {
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
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use proqnt::FromProquints;
        u128::parse_proquints(s)
            .map(From::from)
            .map(Self::new)
            .map_err(|_| "Invalid proquint")
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
impl Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}{}", self.0[0], self.0[1], self.0[2])
    }
}
impl FromStr for Currency {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let c: [char; 3] = s.chars().collect::<Vec<_>>().try_into().map_err(|_| "Requires exactly 3 upper-case chars")?;
        if !c.iter().all(|x| x.is_ascii_uppercase()) { return Err("Requires exactly 3 upper-case chars"); }
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
impl Display for Amount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:02} {}", self.0 / 100, self.0 % 100, self.1)
    }
}
impl FromStr for Amount {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let e = Err("Amounts of currency are formatted as XXXX.XX CC");
        let Some((amount, currency)) = s.split_once(' ') else { return e };
        let Some((whole, cents)) = amount.split_once('.') else { return e };
        if !(whole.chars().all(|c| c.is_ascii_digit()) && cents.chars().all(|c| c.is_ascii_digit())) {
            return e
        }
        if cents.chars().count() != 2 { return e }
        Ok(Self(whole.parse::<i32>().unwrap() * 100 + cents.parse::<i32>().unwrap(), currency.parse()?))
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

impl Amounts {
    pub fn add(&mut self, amount: Amount) -> Amount {
        let present = self.0.entry(amount.1).or_insert(Amount(0, amount.1));
        assert!(present.1 == amount.1);
        present.0 += amount.0;
        *present
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Physical;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Virtual;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountType {
    Physical,
    Virtual,
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

impl<T> Account<T> {
    fn change_type<U>(self, typ: U) -> Account<U> {
        let Account {
            id,
            name,
            notes,
            typ: _,
            current,
            enabled,
        } = self;
        Account {
            id: id.erase().unerase(),
            name,
            notes,
            typ,
            current,
            enabled,
        }
    }
}

impl Account<Physical> {
    fn erase(self) -> Account<AccountType> {
        self.change_type(AccountType::Physical)
    }
    fn from_erased(t: Account<AccountType>) -> Option<Self> {
        if let AccountType::Physical = t.typ {
            Some(t.change_type(Physical))
        } else {
            None
        }
    }
}
impl Account<Virtual> {
    fn erase(self) -> Account<AccountType> {
        self.change_type(AccountType::Virtual)
    }
    fn from_erased(t: Account<AccountType>) -> Option<Self> {
        if let AccountType::Virtual = t.typ {
            Some(t.change_type(Virtual))
        } else {
            None
        }
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
        fees: i32,
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
