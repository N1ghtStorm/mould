#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Statement {
    Let(LetStatement),
    Call(CallStatement),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LetStatement {
    pub name: String,
    pub ty: Type,
    pub value: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallStatement {
    pub name: String,
    pub arguments: Vec<Expression>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    I128,
    U128,
    Isize,
    Usize,
    F16,
    F32,
    F64,
    Bool,
}

impl Type {
    pub const ALL: [Self; 16] = [
        Self::I8,
        Self::U8,
        Self::I16,
        Self::U16,
        Self::I32,
        Self::U32,
        Self::I64,
        Self::U64,
        Self::I128,
        Self::U128,
        Self::Isize,
        Self::Usize,
        Self::F16,
        Self::F32,
        Self::F64,
        Self::Bool,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "i8" => Some(Self::I8),
            "u8" => Some(Self::U8),
            "i16" => Some(Self::I16),
            "u16" => Some(Self::U16),
            "i32" => Some(Self::I32),
            "u32" => Some(Self::U32),
            "i64" => Some(Self::I64),
            "u64" => Some(Self::U64),
            "i128" => Some(Self::I128),
            "u128" => Some(Self::U128),
            "isize" => Some(Self::Isize),
            "usize" => Some(Self::Usize),
            "f16" => Some(Self::F16),
            "f32" => Some(Self::F32),
            "f64" => Some(Self::F64),
            "bool" => Some(Self::Bool),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::U8 => "u8",
            Self::I16 => "i16",
            Self::U16 => "u16",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::I64 => "i64",
            Self::U64 => "u64",
            Self::I128 => "i128",
            Self::U128 => "u128",
            Self::Isize => "isize",
            Self::Usize => "usize",
            Self::F16 => "f16",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bool => "bool",
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::I128
                | Self::U128
                | Self::Isize
                | Self::Usize
        )
    }

    pub fn is_float(self) -> bool {
        matches!(self, Self::F16 | Self::F32 | Self::F64)
    }

    pub fn is_bool(self) -> bool {
        self == Self::Bool
    }

    pub fn max_integer_value(self) -> Option<u128> {
        let bits = match self {
            Self::I8 => 7,
            Self::U8 => 8,
            Self::I16 => 15,
            Self::U16 => 16,
            Self::I32 => 31,
            Self::U32 => 32,
            Self::I64 | Self::Isize => 63,
            Self::U64 | Self::Usize => 64,
            Self::I128 => 127,
            Self::U128 => 128,
            Self::F16 | Self::F32 | Self::F64 | Self::Bool => return None,
        };

        if bits == 128 {
            Some(u128::MAX)
        } else {
            Some((1u128 << bits) - 1)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expression {
    Integer(u128),
    Float(String),
    Bool(bool),
    Variable(String),
}
