/*
 * Copyright (c) 2017 Boucher, Antoni <bouanto@zoho.com>
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy of
 * this software and associated documentation files (the "Software"), to deal in
 * the Software without restriction, including without limitation the rights to
 * use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
 * the Software, and to permit persons to whom the Software is furnished to do so,
 * subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
 * FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
 * COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
 * IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
 * CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
 */

use std::fmt::{self, Display, Formatter};

use literalext::LiteralExt;
use syn::{
    self,
    AngleBracketedGenericArguments,
    ExprKind,
    GenericArgument,
    Lit,
    LitKind,
    Path,
    PathArguments,
    TypePath,
    parse,
};

use ast::Expression;
use gen::ToSql;
use state::{get_primary_key_field, tables_singleton};

/// A field type.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Type {
    Bool,
    ByteString,
    Char,
    Custom(String),
    F32,
    F64,
    Generic,
    I8,
    I16,
    I32,
    I64,
    LocalDateTime,
    NaiveDate,
    NaiveDateTime,
    NaiveTime,
    Nullable(Box<Type>),
    Serial,
    String,
    UnsupportedType(String),
    UtcDateTime,
}

impl Type {
    // TODO: not sure it's a good idea. We already lost the Span.
    pub fn to_syn(&self) -> syn::Type {
        let code =
            match *self {
                Type::I32 => {
                    quote! { i32 }
                },
                Type::I64 => {
                    quote! { i64 }
                },
                Type::String => {
                    quote! { String }
                },
                _ => unimplemented!("Type::to_syn({:?})", self),
            };
        parse(code.into()).expect("parse to_syn()")
    }
}

impl Display for Type {
    /// Get a string representation of the SQL `Type` for display in error messages.
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let typ = match *self {
            Type::Bool => "bool".to_string(),
            Type::ByteString => "Vec<u8>".to_string(),
            Type::Char => "char".to_string(),
            Type::Custom(ref typ) => typ.clone(),
            Type::F32 => "f32".to_string(),
            Type::F64 => "f64".to_string(),
            Type::Generic => "".to_string(),
            Type::I8 => "i8".to_string(),
            Type::I16 => "i16".to_string(),
            Type::I32 => "i32".to_string(),
            Type::I64 => "i64".to_string(),
            Type::LocalDateTime => "chrono::datetime::DateTime<chrono::offset::Local>".to_string(),
            Type::NaiveDate => "chrono::naive::NaiveDate".to_string(),
            Type::NaiveDateTime => "chrono::naive::NaiveDateTime".to_string(),
            Type::NaiveTime => "chrono::naive::NaiveTime".to_string(),
            Type::Nullable(ref typ) => "Option<".to_string() + &typ.to_string() + ">",
            Type::Serial => "i32".to_string(),
            Type::String => "String".to_string(),
            Type::UnsupportedType(_) => "".to_string(),
            Type::UtcDateTime => "chrono::datetime::DateTime<chrono::offset::Utc>".to_string(),
        };
        write!(f, "{}", typ)
    }
}

/// Convert a `Type` to its SQL representation.
fn type_to_sql(typ: &Type, mut nullable: bool) -> String {
    let sql_type =
        match *typ {
            Type::Bool => "BOOLEAN".to_string(),
            Type::ByteString => "BYTEA".to_string(),
            Type::I8 | Type::Char => "CHARACTER(1)".to_string(),
            Type::Custom(ref related_table_name) => {
                let tables = tables_singleton();
                if let Some(table) = tables.get(related_table_name) {
                    let primary_key_field: String = get_primary_key_field(table).unwrap().to_string();
                    "INTEGER REFERENCES ".to_string() + &related_table_name + "(" + &primary_key_field + ")"
                }
                else {
                    "".to_string()
                }
                // NOTE: if the field type is not an SQL table, an error is thrown by the linter.
            },
            Type::F32 => "REAL".to_string(),
            Type::F64 => "DOUBLE PRECISION".to_string(),
            Type::Generic => "".to_string(),
            Type::I16 => "SMALLINT".to_string(),
            Type::I32 => "INTEGER".to_string(),
            Type::I64 => "BIGINT".to_string(),
            Type::LocalDateTime => "TIMESTAMP WITH TIME ZONE".to_string(),
            Type::NaiveDate => "DATE".to_string(),
            Type::NaiveDateTime => "TIMESTAMP".to_string(),
            Type::NaiveTime => "TIME".to_string(),
            Type::Nullable(ref typ) => {
                nullable = true;
                type_to_sql(&*typ, true)
            },
            Type::Serial => "SERIAL PRIMARY KEY".to_string(),
            Type::String => "CHARACTER VARYING".to_string(),
            Type::UnsupportedType(_) => "".to_string(), // TODO: should panic.
            Type::UtcDateTime => "TIMESTAMP WITH TIME ZONE".to_string(),
        };

    if nullable {
        sql_type
    }
    else {
        sql_type + " NOT NULL"
    }
}

impl ToSql for Type {
    fn to_sql(&self) -> String {
        type_to_sql(self, false)
    }
}

impl PartialEq<Expression> for Type {
    /// Check if an literal `expression` is equal to a `Type`.
    fn eq(&self, expression: &Expression) -> bool {
        // If the field type is `Nullable`, `expected_type` needs not to be an `Option`.
        let typ =
            match *self {
                Type::Nullable(ref typ) => typ,
                ref typ => typ,
            };
        match expression.node {
            ExprKind::Lit(Lit { value: LitKind::Bool(_), .. }) => *typ == Type::Bool,
            ExprKind::Lit(Lit { value: LitKind::Other(ref literal), .. }) => {
                if literal.parse_byte().is_some() {
                    false
                }
                else if literal.parse_bytes().is_some() {
                    *typ == Type::ByteString
                }
                else if literal.parse_char().is_some() {
                    *typ == Type::Char
                }
                else if let Some(float) = literal.parse_float() {
                    match float.suffix() {
                        // TODO: check if right suffix.
                        "f32" => *typ == Type::F32,
                        "f64" => *typ == Type::F64,
                        suffix if suffix.is_empty() => *typ == Type::F32 || *typ == Type::F64,
                        _ => panic!("Unexpected float suffix {}", float.suffix()),
                    }
                }
                else if literal.parse_string().is_some() {
                    *typ == Type::String
                }
                else if let Some(int) = literal.parse_int() {
                    match int.suffix() {
                        "isize" => false,
                        "i8" => *typ == Type::I8,
                        "i16" => *typ == Type::I16,
                        "i32" => *typ == Type::I32 || *typ == Type::Serial,
                        "i64" => *typ == Type::I64,
                        "u8" | "u16" | "u32" | "u64" => false,
                        suffix if suffix.is_empty() =>
                            *typ == Type::I8 ||
                            *typ == Type::I16 ||
                            *typ == Type::I32 ||
                            *typ == Type::I64 ||
                            *typ == Type::Serial,
                        _ => panic!("Unexpected int suffix {}", int.suffix()),
                    }
                }
                else {
                    // FIXME: workaround for stable.
                    #[cfg(not(unstable))]
                    {
                        let string = literal.to_string();
                        if string == "true" || string == "false" {
                            return *typ == Type::Bool;
                        }
                    }
                    unreachable!("types: unexpected literal type");
                }
            }
            _ => true, // Returns true, because the type checking for non-literal is done later.
        }
    }
}

impl<'a> From<&'a Path> for Type {
    /// Convert a `Path` to a `Type`.
    fn from(&Path { ref segments, .. }: &Path) -> Type {
        let unsupported = Type::UnsupportedType("".to_string());
        if segments.len() == 1 {
            let element = segments.first().expect("first segment of path");
            let first_segment = element.item();
            let ident = first_segment.ident.to_string();
            match &ident[..] {
                "bool" => Type::Bool,
                "char" => Type::Char,
                "DateTime" => match get_type_parameter(&first_segment.arguments) {
                    Some(ty) => match ty.as_ref() {
                        "Local" => Type::LocalDateTime,
                        "Utc" => Type::UtcDateTime,
                        parameter_type => Type::UnsupportedType("DateTime<".to_string() + parameter_type + ">"),
                    },
                    None => Type::UnsupportedType("DateTime".to_string()),
                },
                "f32" => Type::F32,
                "f64" => Type::F64,
                "i8" => Type::I8,
                "i16" => Type::I16,
                "i32" => Type::I32,
                "i64" => Type::I64,
                "ForeignKey" => match get_type_parameter(&first_segment.arguments) {
                    Some(ty) => Type::Custom(ty),
                    None => Type::UnsupportedType("ForeignKey".to_string()),
                },
                "NaiveDate" => Type::NaiveDate,
                "NaiveDateTime" => Type::NaiveDateTime,
                "NaiveTime" => Type::NaiveTime,
                "Option" =>
                    match get_type_parameter_as_path(&first_segment.arguments) {
                        Some(ty) => {
                            let result = From::from(ty);
                            let typ =
                                if let Type::Nullable(_) = result {
                                    Type::UnsupportedType(result.to_string())
                                }
                                else {
                                    From::from(ty)
                                };
                            Type::Nullable(Box::new(typ))
                        },
                        None => Type::UnsupportedType("Option".to_string()),
                    },
                "PrimaryKey" => {
                    Type::Serial
                },
                "String" => {
                    Type::String
                },
                "Vec" => match get_type_parameter(&first_segment.arguments) {
                    Some(ty) => match ty.as_ref() {
                        "u8" => Type::ByteString,
                        parameter_type => Type::UnsupportedType("Vec<".to_string() + parameter_type + ">"),
                    },
                    None => Type::UnsupportedType("Vec".to_string()),
                },
                typ => Type::UnsupportedType(typ.to_string()), // TODO: show the generic types as well.
            }
        }
        else {
            unsupported
        }
    }
}

/// Get the type between < and > as a String.
fn get_type_parameter(parameters: &PathArguments) -> Option<String> {
    get_type_parameter_as_path(parameters).map(|path| path.segments.first()
        .expect("first segment in path").item().ident.to_string())
}

/// Get the type between < and > as a Path.
fn get_type_parameter_as_path(parameters: &PathArguments) -> Option<&Path> {
    if let PathArguments::AngleBracketed(AngleBracketedGenericArguments { ref args, .. }) = *parameters {
        args.first()
            .and_then(|ty| {
                if let GenericArgument::Type(syn::Type::Path(TypePath { ref path, .. })) = **ty.item() {
                    Some(path)
                }
                else {
                    None
                }
            })
    }
    else {
        None
    }
}
