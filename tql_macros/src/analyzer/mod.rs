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

//! Semantic analyzer.

mod aggregate;
mod assignment;
mod filter;
mod get;
mod insert;
mod join;
mod limit;
mod sort;

use std::borrow::Cow;
use std::fmt::Display;
use std::result;

use literalext::LiteralExt;
use syn::{
    Expr,
    ExprKind,
    ExprPath,
    Ident,
    Lit,
    LitKind,
    Span,
};
use syn::delimited::Delimited;
use syn::tokens::Comma;

use ast::{
    self,
    Aggregate,
    AggregateFilterExpression,
    Assignment,
    Expression,
    FieldList,
    FilterExpression,
    FilterValue,
    Groups,
    Identifier,
    Join,
    Limit,
    Order,
    Query,
    TypedField,
    WithSpan,
    expr_span,
};
use error::{Error, Result, res};
use gen::ToSql;
use parser::{MethodCall, MethodCalls};
use plugin::number_literal;
use self::aggregate::{argument_to_aggregate, argument_to_group, expression_to_aggregate_filter_expression};
use self::assignment::{analyze_assignments_types, argument_to_assignment};
use self::filter::{analyze_filter_types, expression_to_filter_expression};
use self::get::get_expression_to_filter_expression;
use self::insert::check_insert_arguments;
use self::join::argument_to_join;
use self::limit::{analyze_limit_types, argument_to_limit};
use self::sort::argument_to_order;
use state::{SqlTable, SqlTables, get_field_type, methods_singleton, tables_singleton};
use string::{find_near, plural_verb};
use types::Type;

/// The type of the SQL query.
enum SqlQueryType {
    Aggregate,
    CreateTable,
    Delete,
    Drop,
    Insert,
    Select,
    Update,

}

impl Default for SqlQueryType {
    fn default() -> SqlQueryType {
        SqlQueryType::Select
    }
}

/// The query data gathered during the analysis.
#[derive(Default)]
// TODO: improve this design. It should not be necessary to hold data that are not needed for a
// specific query.
struct QueryData {
    // Aggregate
    aggregate_filter: AggregateFilterExpression,
    aggregates: Vec<Aggregate>,
    groups: Groups,
    // Aggregate, Delete, Select, Update
    filter: FilterExpression,
    // Aggregate / Select
    joins: Vec<Join>,
    // Create
    fields_to_create: Vec<TypedField>,
    // Insert / Update
    assignments: Vec<Assignment>,
    // Select
    fields: FieldList,
    limit: Limit,
    order: Vec<Order>,
    // All
    query_type: SqlQueryType,
}

/// Analyze and transform the AST.
pub fn analyze(method_calls: MethodCalls, sql_tables: &SqlTables) -> Result<Query> {
    let mut errors = vec![];

    // Check if the table exists.
    let table_name = method_calls.name.expect("table name in method_calls").to_string();
    if !sql_tables.contains_key(&table_name) {
        unknown_table_error(&table_name, method_calls.position, sql_tables, &mut errors);
    }

    check_methods(&method_calls, &mut errors);
    check_method_calls_validity(&method_calls, &mut errors);

    let table = sql_tables.get(&table_name);
    let calls = &method_calls.calls;
    let mut delete_position = None;

    // Get all the data from the query.
    let query_data =
        match table {
            Some(table) => {
                let mut query_data = process_methods(&calls, table, &mut delete_position)?;
                let fields = get_query_fields(table, &query_data.joins, sql_tables);
                query_data.fields = fields;
                query_data

            },
            None => QueryData::default(),
        };

    let query = new_query(query_data, table_name);

    check_delete_without_filters(&query, delete_position, &mut errors);

    res(query, errors)
}

/// Analyze the literal types in the `Query`.
pub fn analyze_types(query: Query) -> Result<Query> {
    let mut errors = vec![];
    match query {
        Query::Aggregate { ref filter, ref table, .. } => {
            analyze_filter_types(filter, &table, &mut errors);
        },
        Query::CreateTable { .. } => (), // Nothing to analyze.
        Query::Delete { ref filter, ref table } => {
            analyze_filter_types(filter, &table, &mut errors);
        },
        Query::Drop { .. } => (), // Nothing to analyze.
        Query::Insert { ref assignments, ref table } => {
            analyze_assignments_types(assignments, &table, &mut errors);
        },
        Query::Select { ref filter, ref limit, ref table, .. } => {
            analyze_filter_types(filter, &table, &mut errors);
            analyze_limit_types(limit, &mut errors);
        },
        Query::Update { ref assignments, ref filter, ref table } => {
            analyze_filter_types(filter, &table, &mut errors);
            analyze_assignments_types(assignments, &table, &mut errors);
        },
    }
    res(query, errors)
}

/// Check that the `arguments` vector contains `expected_count` elements.
/// If this is not the case, add an error to `errors`.
fn check_argument_count(arguments: &Delimited<Expr, Comma>, expected_count: usize, position: Span, errors: &mut Vec<Error>) -> bool {
    if arguments.len() == expected_count {
        true
    }
    else {
        let length = arguments.len();
        errors.push(Error::new_with_code(
            // FIXME: the position should be all the arguments, instead of the function name.
            &format!("this function takes 1 parameter but {} parameter{} supplied", length, plural_verb(length)),
            position,
            "E0061",
        ));
        false
    }
}

/// Check that `Delete` `Query` contains a filter.
fn check_delete_without_filters(query: &Query, delete_position: Option<Span>, errors: &mut Vec<Error>) {
    if let Query::Delete { ref filter, .. } = *query {
        if let FilterExpression::NoFilters = *filter {
            errors.push(Error::new_warning(
                "delete() without filters",
                delete_position.unwrap(), // There is always a delete position when the query is of type Delete.
            ));
        }
    }
}

/// Check if the `identifier` is a field in the struct `table_name`.
pub fn check_field(identifier: &Ident, position: Span, table: &SqlTable, errors: &mut Vec<Error>) {
    if !table.fields.contains_key(identifier) {
        let field_names = table.fields.keys()
            .map(|ident| ident.as_ref());
        let mut error = Error::new(
            &format!("attempted access of field `{field}` on type `{table}`, but no field with that name was found",
                field = identifier,
                table = table.name
            ),
            position
        );
        propose_similar_name(identifier.as_ref(), field_names, &mut error);
        errors.push(error);
    }
}

/// Check if the type of `identifier` matches the type of the `value` expression.
fn check_field_type(table_name: &str, filter_value: &FilterValue, value: &Expression, errors: &mut Vec<Error>) {
    let field_type = get_field_type_by_filter_value(table_name, filter_value);
    check_type(field_type, value, errors);
}

/// Check if the method calls sequence is valid.
/// For instance, one cannot call both insert() and delete() methods in the same query.
fn check_method_calls_validity(method_calls: &MethodCalls, errors: &mut Vec<Error>) {
    let method_map =
        hashmap!{
            "aggregate" => vec!["filter", "join", "values"],
            "all" => vec!["filter", "get", "join", "limit", "sort"],
            "create" => vec![],
            "delete" => vec!["filter", "get"],
            "drop" => vec![],
            "insert" => vec![],
            "update" => vec!["filter", "get"],
        };

    let main_method = method_calls.calls.iter()
        .filter(|call| method_map.contains_key(&*call.name) )
        .next()
        .map_or("all", |call| call.name.as_str());

    // TODO: check that the insert, update or delete methods are not called more than once.
    let mut valid_methods = vec![main_method];
    valid_methods.append(&mut method_map[&main_method].clone());

    let methods = get_methods();
    let invalid_methods = method_calls.calls.iter()
        .filter(|call| methods.contains(&call.name) && !valid_methods.contains(&&*call.name));

    for method in invalid_methods {
        errors.push(Error::new(
            &format!("cannot call the {method}() method with the {main_method}() method",
                method = method.name,
                main_method = main_method
            ),
            method.position,
        ));
    }
}

/// Check if the method `calls` exist.
fn check_methods(method_calls: &MethodCalls, errors: &mut Vec<Error>) {
    let methods = get_methods();
    for method_call in &method_calls.calls {
        if !methods.contains(&method_call.name) {
            let mut error = Error::new(
                &format!("no method named `{}` found in tql", method_call.name),
                method_call.position,
            );
            propose_similar_name(&method_call.name, methods.iter().map(String::as_ref), &mut error);
            errors.push(error);
        }
    }

    if method_calls.calls.is_empty() {
        let table_name = &method_calls.name.expect("table name in method_calls");
        let mut error =
            Error::new_with_code(
                &format!("`{table}` is the name of a struct, but this expression uses it like a method name",
                        table = table_name
                       ),
                method_calls.position, "E0423"
            );
        error.add_help(&format!("did you mean to write `{}.method()`?", table_name));
        errors.push(error);
    }
}

/// Check that the specified method call did not received any arguments.
fn check_no_arguments(method_call: &MethodCall, errors: &mut Vec<Error>) {
    if !method_call.args.is_empty() {
        let length = method_call.args.len();
        errors.push(Error::new_with_code(
            &format!("this method takes 0 parameters but {param_count} parameter{plural} supplied",
                    param_count = length,
                    plural = plural_verb(length)
                   ),
            method_call.position, "E0061"
       ));
    }
}

/// Check if the `field_type` is compatible with the `expression`'s type.
pub fn check_type(field_type: &Type, expression: &Expression, errors: &mut Vec<Error>) {
    if field_type != expression {
        let literal_type = get_type(expression);
        mismatched_types(field_type, &literal_type, expr_span(expression), errors);
    }
}

/// Check if the `field_type` is compatible with the `filter_value`'s type.
fn check_type_filter_value(expected_type: &Type, filter_value: &WithSpan<FilterValue>, table_name: &str, errors: &mut Vec<Error>) {
    let field_type = get_field_type_by_filter_value(table_name, &filter_value.node);
    if *field_type != *expected_type {
        mismatched_types(expected_type, &field_type, filter_value.span, errors);
    }
}

/// Convert the `arguments` to the `Type`.
fn convert_arguments<F, Type>(arguments: &[Expression], table: &SqlTable, convert_argument: F) -> Result<Vec<Type>>
    where F: Fn(&Expression, &SqlTable) -> Result<Type>
{
    let mut items = vec![];
    let mut errors = vec![];

    for arg in arguments {
        try(convert_argument(arg, table), &mut errors, |item| {
            items.push(item);
        });
    }

    res(items, errors)
}

/// Get the type of the field if it exists from an `FilterValue`.
fn get_field_type_by_filter_value<'a>(table_name: &'a str, filter_value: &FilterValue) -> &'a Type {
    // NOTE: At this stage (type analysis), the field exists, hence unwrap().
    match *filter_value {
        FilterValue::Identifier(ref identifier) => {
            get_field_type(table_name, identifier).unwrap()
        },
        FilterValue::MethodCall(ast::MethodCall { ref method_name, ref object_name, .. }) => {
            let tables = tables_singleton();
            let table = tables.get(table_name).unwrap();
            let methods = methods_singleton();
            let types = table.fields.get(object_name).unwrap();
            let typ =
                match types.ty.node {
                    // NOTE: return a Generic Type because Option methods work independently from
                    // the nullable type (for instance, is_some()).
                    Type::Nullable(_) => Cow::Owned(Type::Nullable(Box::new(Type::Generic))),
                    ref typ => Cow::Borrowed(typ),
                };
            let type_methods = methods.get(&typ).unwrap();
            let method = type_methods.get(method_name).unwrap();
            &method.return_type
        },
        FilterValue::None => unreachable!("FilterValue::None in get_field_type_by_filter_value()"),
    }
}

/// Get all the existing methods.
// TODO: return Vec<&'static str> instead?
fn get_methods() -> Vec<String> {
    vec![
        "aggregate".to_string(),
        "all".to_string(),
        "create".to_string(),
        "delete".to_string(),
        "drop".to_string(),
        "filter".to_string(),
        "get".to_string(),
        "insert".to_string(),
        "join".to_string(),
        "limit".to_string(),
        "sort".to_string(),
        "update".to_string(),
        "values".to_string(),
    ]
}

/// Get the query field fully qualified names.
fn get_query_fields(table: &SqlTable, joins: &[Join], sql_tables: &SqlTables) -> Vec<Identifier> {
    let mut fields = vec![];
    for (field, types) in &table.fields {
        match types.ty.node {
            // TODO: pay attention to name conflicts (join on same table twice).
            Type::Custom(ref foreign_table) => {
                let table_name = foreign_table;
                if let Some(foreign_table) = sql_tables.get(foreign_table) {
                    if has_joins(&joins, field) {
                        for (field, types) in &foreign_table.fields {
                            match types.ty.node {
                                Type::Custom(_) | Type::UnsupportedType(_) => (), // NOTE: Do not add foreign key recursively.
                                _ => {
                                    fields.push(table_name.clone() + "." + field.as_ref());
                                },
                            }
                        }
                    }
                }
                // TODO: Check if the foreign table exists instead of doing this in the lint plugin
                // (it is needed here because the related fields need to be included in the query.)
                // Not sure about this. I think it is ok like this.
            },
            Type::UnsupportedType(_) => (),
            _ => {
                fields.push(table.name.to_string() + "." + field.as_ref());
            },
        }
    }
    fields
}

/// Get the string representation of an literal `Expression` type.
/// Useful to show in an error.
fn get_type(expression: &Expression) -> &str {
    match expression.node {
        ExprKind::Lit(ref literal) => {
            match *literal {
                Lit { value: LitKind::Bool(_), .. } => "bool",
                Lit { value: LitKind::Other(ref literal), .. } => {
                    if let Some(int) = literal.parse_int() {
                        match int.suffix() {
                            "isize" => "isize",
                            "i8" => "i8",
                            "i16" => "i16",
                            "i32" => "i32",
                            "i64" => "i64",
                            "usize" => "usize",
                            "u8" => "u8",
                            "u16" => "u16",
                            "u32" => "u32",
                            "u64" => "u64",
                            "" => "integral variable",
                            _ => unreachable!("unexpected int suffix"),
                        }
                    }
                    else if literal.parse_byte().is_some() {
                        "u8"
                    }
                    else if literal.parse_bytes().is_some() {
                        "Vec<u8>"
                    }
                    else if literal.parse_char().is_some() {
                        "char"
                    }
                    else if literal.parse_string().is_some() {
                        "String"
                    }
                    else if let Some(float) = literal.parse_float() {
                        match float.suffix() {
                            "f32" => "f32",
                            "f64" => "f64",
                            "" => "floating-point variable",
                            _ => unreachable!("unexpected float suffix"),
                        }
                    }
                    else {
                        unreachable!("analyzer: unexpected literal type");
                    }
                },
            }
        }
        _ => panic!("expression needs to be a literal"),
    }
}

/// Check if there is a join in `joins` on a field named `name`.
pub fn has_joins(joins: &[Join], name: &Ident) -> bool {
    joins.iter()
        .map(|join| &join.base_field)
        .any(|field_name| field_name == name.as_ref())
}

/// Add a mismatched types error to `errors`.
fn mismatched_types<S: Display, T: Display>(expected_type: S, actual_type: &T, position: Span, errors: &mut Vec<Error>) {
    let mut error =
        Error::new_with_code(
            &format!("mismatched types:\n expected `{expected_type}`,\n    found `{actual_type}`",
                expected_type = expected_type,
                actual_type = actual_type
            ),
            position,
            "E0308",
        );
    error.add_note("in this expansion of sql! (defined in tql)");
    errors.push(error);
}

/// Create a new query from all the data gathered by the method calls.
fn new_query(QueryData { fields, filter, joins, limit, order, assignments, fields_to_create, aggregates, groups,
    aggregate_filter, query_type }: QueryData, table_name: String) -> Query
{
    match query_type {
        SqlQueryType::Aggregate =>
            Query::Aggregate {
                aggregates: aggregates,
                aggregate_filter: aggregate_filter,
                filter: filter,
                groups: groups,
                joins: joins,
                table: table_name,
            },
        SqlQueryType::CreateTable =>
            Query::CreateTable {
                fields: fields_to_create,
                table: table_name,
            },
        SqlQueryType::Delete =>
            Query::Delete {
                filter: filter,
                table: table_name,
            },
        SqlQueryType::Drop =>
            Query::Drop {
                table: table_name,
            },
        SqlQueryType::Insert =>
            Query::Insert {
                assignments: assignments,
                table: table_name,
            },
        SqlQueryType::Select =>
            Query::Select {
                fields: fields,
                filter: filter,
                joins: joins,
                limit: limit,
                order: order,
                table: table_name,
            },
        SqlQueryType::Update =>
            Query::Update {
                assignments: assignments,
                filter: filter,
                table: table_name,
            },
    }
}

/// Create an error about a table not having a primary key.
pub fn no_primary_key(table_name: &str, position: Span) -> Error {
    Error::new(
        &format!("Table {table} does not have a primary key", // TODO: improve this message.
            table = table_name
        ),
        position
    )
}

/// Convert an `Expression` to an `Ident` if `expression` is an `ExprPath`.
/// It adds an error to `errors` if `expression` is not an `ExprPath`.
fn path_expr_to_identifier(expression: &Expression, errors: &mut Vec<Error>) -> Option<Ident> {
    if let ExprKind::Path(ExprPath { ref path, .. }) = expression.node {
        let identifier = path.segments.first().unwrap().into_item().ident.clone();
        Some(identifier)
    }
    else {
        errors.push(Error::new(
            "Expected identifier", // TODO: improve this message.
            expr_span(expression),
        ));
        None
    }
}

/// Convert an `Expression` to a `String` if `expression` is an `ExprPath`.
/// It adds an error to `errors` if `expression` is not an `ExprPath`.
fn path_expr_to_string(expression: &Expression, errors: &mut Vec<Error>) -> Option<String> {
    path_expr_to_identifier(expression, errors)
        .map(|ident| ident.to_string())
}

/// Gather data about the query in the method `calls`.
/// Also analyze the types.
fn process_methods(calls: &[MethodCall], table: &SqlTable, delete_position: &mut Option<Span>) -> Result<QueryData> {
    let mut errors = vec![];
    let mut query_data = QueryData::default();

    for method_call in calls {
        match &method_call.name[..] {
            "aggregate" => {
                try(convert_arguments(&method_call.args, table, argument_to_aggregate), &mut errors, |aggrs| {
                    query_data.aggregates = aggrs;
                });
                query_data.query_type = SqlQueryType::Aggregate;
            },
            "all" => {
                check_no_arguments(&method_call, &mut errors);
            },
            "create" => {
                check_no_arguments(&method_call, &mut errors);
                query_data.query_type = SqlQueryType::CreateTable;
                for (field, types) in &table.fields {
                    query_data.fields_to_create.push(TypedField {
                        identifier: field.to_string(),
                        typ: types.ty.node.to_sql(),
                    });
                }
            },
            "delete" => {
                check_no_arguments(&method_call, &mut errors);
                query_data.query_type = SqlQueryType::Delete;
                *delete_position = Some(method_call.position);
            },
            "drop" => {
                check_no_arguments(&method_call, &mut errors);
                query_data.query_type = SqlQueryType::Drop;
            },
            "filter" => {
                if query_data.aggregates.is_empty() {
                    // If the aggregate() method was not called, filter() filters on the values
                    // (WHERE).
                    try(expression_to_filter_expression(&method_call.args[0], table), &mut errors, |filter| {
                        query_data.filter = filter;
                    });
                }
                else {
                    // If the aggregate() method was called, filter() filters on the aggregated
                    // values (HAVING).
                    try(expression_to_aggregate_filter_expression(&method_call.args[0], &query_data.aggregates, table), &mut errors, |filter| {
                        query_data.aggregate_filter = filter;
                    });
                }
            },
            "get" => {
                if method_call.args.is_empty() {
                    query_data.limit = Limit::Index(number_literal(0));
                }
                else {
                    try(get_expression_to_filter_expression(&method_call.args[0], table), &mut errors, |(filter, new_limit)| {
                        query_data.filter = filter;
                        query_data.limit = new_limit;
                    });
                }
            },
            "insert" => {
                try(convert_arguments(&method_call.args, table, argument_to_assignment), &mut errors, |assigns| {
                    query_data.assignments = assigns;
                });
                if !query_data.assignments.is_empty() {
                    // TODO: check even if there are errors in the assignation types.
                    check_insert_arguments(&query_data.assignments, method_call.position, &table, &mut errors);
                }
                query_data.query_type = SqlQueryType::Insert;
            },
            "join" => {
                try(convert_arguments(&method_call.args, table, argument_to_join), &mut errors, |mut new_joins| {
                    query_data.joins.append(&mut new_joins);
                });
            },
            "limit" => {
                try(argument_to_limit(&method_call.args[0]), &mut errors, |new_limit| {
                    query_data.limit = new_limit;
                });
            },
            "sort" => {
                try(convert_arguments(&method_call.args, table, argument_to_order), &mut errors, |new_order| {
                    query_data.order = new_order;
                });
            },
            "update" => {
                try(convert_arguments(&method_call.args, table, argument_to_assignment), &mut errors, |assigns| {
                    query_data.assignments = assigns;
                });
                query_data.query_type = SqlQueryType::Update;
            },
            "values" => {
                try(convert_arguments(&method_call.args, table, argument_to_group), &mut errors, |new_groups| {
                    query_data.groups = new_groups;
                });
            },
            _ => (), // NOTE: Nothing to do since check_methods() check for unknown method.
        }
    }
    res(query_data, errors)
}

/// Check if a name similar to `identifier` exists in `choices` and show a message if one exists.
/// Returns true if a similar name was found.
pub fn propose_similar_name<'a, T>(identifier: &str, choices: T, error: &mut Error) -> bool
    where T: Iterator<Item = &'a str>
{
    if let Some(name) = find_near(&identifier, choices) {
        error.add_help(&format!("did you mean {}?", name));
        true
    }
    else {
        false
    }
}

/// If `result` is an `Err`, add the errors to `errors`.
/// Otherwise, execute the closure.
fn try<F: FnMut(T), T>(mut result: result::Result<T, Vec<Error>>, errors: &mut Vec<Error>, mut fn_using_result: F) {
    match result {
        Ok(value) => fn_using_result(value),
        Err(ref mut errs) => errors.append(errs),
    }
}

/// Add an error to the vector `errors` about an unknown SQL table.
/// It suggests a similar name if there is one.
pub fn unknown_table_error(table_name: &str, position: Span, sql_tables: &SqlTables, errors: &mut Vec<Error>) {
    let mut error = Error::new_with_code(
        &format!("`{table}` does not name an SQL table",
                table = table_name
        ),
        position,
        "E0422",
    );
    let tables = sql_tables.keys().map(String::as_ref);
    if !propose_similar_name(&table_name, tables, &mut error) {
        error.add_help(
            &format!("did you forget to add the #[derive(SqlTable)] attribute on the {} struct?", table_name),
        );
    }
    errors.push(error);
}
