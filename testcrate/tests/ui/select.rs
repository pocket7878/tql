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

//! Tests of the methods related to `Query::Select`.

#![feature(proc_macro)]

extern crate tql;
#[macro_use]
extern crate tql_macros;

use tql::PrimaryKey;
use tql_macros::sql;

struct Connection {
    value: String,
}

#[derive(SqlTable)]
struct Table {
    id: PrimaryKey,
    field1: String,
    i32_field: i32,
}

fn main() {
    sql!(Table.filter(field1 == "value1" && field2 < 100).sort(-field2));
    //~^ ERROR attempted access of field `field2` on type `Table`, but no field with that name was found
    //~| HELP did you mean field1?
    //~| ERROR attempted access of field `field2` on type `Table`, but no field with that name was found
    //~| HELP did you mean field1?

    sql!(Table.filter(field1 == "value1" && i32_field < 100u32));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `u32`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(field1 == "value1" && i32_field < 100u32).sort(-i32_field));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `u32`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(field1 == "value1" && i32_field < 100u64));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `u64`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(field1 == "value1" && i32_field < 100i8));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `i8`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= b'f' || field1 == 't'));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `u8`
    //~| NOTE in this expansion of sql! (defined in tql)
    //~| ERROR mismatched types:
    //~| expected `String`,
    //~| found `char`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= b"test"));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `Vec<u8>`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= r#""test""#));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `String`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= 3.141592f32));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `f32`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= 3.141592f64));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `f64`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= 3.141592));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `floating-point variable`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field >= 42).sort(fild1));
    //~^ ERROR attempted access of field `fild1` on type `Table`, but no field with that name was found
    //~| HELP did you mean field1?

    sql!(Table.filter(i32_field >= 42).sort(-fild1));
    //~^ ERROR attempted access of field `fild1` on type `Table`, but no field with that name was found
    //~| HELP did you mean field1?

    sql!(Table.filter(i32_fiel >= 42));
    //~^ ERROR attempted access of field `i32_fiel` on type `Table`, but no field with that name was found
    //~| HELP did you mean i32_field?

    sql!(Table.filter(i32_field == true));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `bool`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.filter(i32_field == false));
    //~^ ERROR mismatched types:
    //~| expected `i32`,
    //~| found `bool`
    //~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.all()[.."auinesta"]);
    // ~^ ERROR mismatched types:
    // ~| expected `i64`,
    // ~| found `String`
    // ~| NOTE in this expansion of sql! (defined in tql)

    sql!(Table.all()[true..false]);
    // ~^ ERROR mismatched types:
    // ~| expected `i64`,
    // ~| found `bool`
    // ~| NOTE in this expansion of sql! (defined in tql)
    // ~| ERROR mismatched types:
    // ~| expected `i64`,
    // ~| found `bool`
    // ~| NOTE in this expansion of sql! (defined in tql)
    // FIXME: the position should be on the star for the next sql!() query.

    sql!(Table.filter(i32_field < 100 && field1 == "value1").sort(*i32_field, *field1));
    //~^ ERROR Expected - or identifier
    //~| ERROR Expected - or identifier

    sql!(Tble.filter(field1 == "value"));
    //~^ ERROR `Tble` does not name an SQL table
    //~| HELP did you mean Table?

    sql!(TestTable.flter(field1 == "value"));
    //~^ ERROR `TestTable` does not name an SQL table
    //~| HELP did you forget to add the #[derive(SqlTable)] attribute on the TestTable struct?
    //~| ERROR no method named `flter` found in tql
    //~| HELP did you mean filter?

    sql!(Table.all(id == 1));
    //~^ ERROR this method takes 0 parameters but 1 parameter was supplied

    sql!(Table.all().join(test));
    //~^ ERROR attempted access of field `test` on type `Table`, but no field with that name was found

    sql!(Table.all().join(field));
    //~^ ERROR attempted access of field `field` on type `Table`, but no field with that name was found
    //~| HELP did you mean field1?

    sql!(Table.all().join(field1, i32_field));
    //~^ ERROR mismatched types:
    //~| expected `ForeignKey<_>`,
    //~| found `String`
    //~| NOTE in this expansion of sql! (defined in tql)
    //~| ERROR mismatched types:
    //~| expected `ForeignKey<_>`,
    //~| found `i32`
    //~| NOTE in this expansion of sql! (defined in tql)

    //to_sql!(Table.all().join(address, address)); // TODO: should span an error.
}
