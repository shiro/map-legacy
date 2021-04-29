use anyhow::*;
use evdev_rs::enums::EventType;
use futures::StreamExt;
use nom::branch::*;
use nom::bytes::complete::*;
use nom::character::complete::*;
use nom::combinator::{map, opt};
use nom::Err as NomErr;
use nom::error::{context, VerboseError};
use nom::IResult;
use nom::multi::many0;
use nom::sequence::*;
use tap::Tap;

use variable::*;
use identifier::*;
use lambda::*;
use key_mapping::*;
use key::*;
use key_action::*;
use primitives::*;
use function::*;
use key_sequence::*;

use crate::*;
use crate::block_ext::ExprVecExt;
use crate::parsing::custom_combinators::fold_many0_once;
use crate::parsing::identifier::ident;

pub mod parser;
mod key_sequence;
mod custom_combinators;
mod function;
mod identifier;
mod key;
mod key_action;
mod key_mapping;
mod lambda;
mod primitives;
mod variable;

type Res<T, U> = IResult<T, U, VerboseError<T>>;

fn make_generic_nom_err<'a>() -> NomErr<VerboseError<&'a str>> { NomErr::Error(VerboseError { errors: vec![] }) }


fn expr_simple(input: &str) -> Res<&str, Expr> {
    context(
        "expr_simple",
        tuple((
            alt((
                boolean,
                string,
                lambda,
                variable_initialization,
                variable_assignment,
                function_call,
                key_mapping_inline,
                key_mapping,
                variable,
            )),
            multispace0,
        )),
    )(input).map(|(next, v)| (next, v.0))
}

fn expr(i: &str) -> Res<&str, Expr> {
    let (i, init) = expr_simple(i)?;
    fold_many0_once(
        |i: &str| {
            context(
                "expr",
                tuple((
                    multispace0,
                    alt((tag("=="), tag("!="))),
                    multispace0,
                    expr_simple,
                )),
            )(i)
        },
        init,
        |acc, (_, op, _, val)| {
            match op {
                "==" => Expr::Eq(Box::new(acc), Box::new(val)),
                // TODO implement neq
                "!=" => Expr::Eq(Box::new(acc), Box::new(val)),
                _ => unreachable!()
            }
        },
    )(i)
}

fn if_stmt(input: &str) -> Res<&str, Stmt> {
    context(
        "if_stmt",
        tuple((
            tag("if"),
            multispace0,
            tag("("),
            multispace0,
            expr,
            multispace0,
            tag(")"),
            multispace0,
            block,
        )),
    )(input).map(|(next, v)| (next, Stmt::If(v.4, v.8)))
}


fn stmt(input: &str) -> Res<&str, Stmt> {
    context(
        "stmt",
        tuple((
            alt((
                if_stmt,
                map(tuple((expr, tag(";"))), |v| Stmt::Expr(v.0)),
                map(block, Stmt::Block),
            )),
        )),
    )(input).map(|(next, val)| (next, val.0))
}

fn block_body(input: &str) -> Res<&str, Block> {
    context(
        "block_body",
        opt(tuple((
            stmt,
            many0(tuple((
                multispace0,
                stmt,
            ))),
        ))),
    )(input).map(|(next, v)| {
        match v {
            Some((s1, s2)) => {
                (next, Block::new().tap_mut(|b| {
                    let mut statements: Vec<Stmt> = s2.into_iter().map(|x| x.1).collect();
                    statements.insert(0, s1);
                    b.statements = statements;
                }))
            }
            _ => (next, Block::new())
        }
    })
}

fn block(input: &str) -> Res<&str, Block> {
    context(
        "block",
        tuple((
            tag("{"),
            multispace0,
            block_body,
            multispace0,
            tag("}")
        )),
    )(input).map(|(next, v)| (next, v.2))
}

fn global_block(input: &str) -> Res<&str, Block> {
    context(
        "block",
        tuple((multispace0, block_body, multispace0)),
    )(input).map(|(next, v)| (next, v.1))
}


#[cfg(test)]
mod tests {
    use nom::error::{ErrorKind, VerboseErrorKind};
    use tap::Tap;

    use crate::block_ext::ExprVecExt;

    use super::*;

    #[test]
    fn test_if_stmt() {
        assert_eq!(if_stmt("if(true){ a::b; }"), Ok(("", Stmt::If(
            expr("true").unwrap().1,
            block("{a::b;}").unwrap().1,
        ))));
        assert_eq!(stmt("if(true){ a::b; }"), Ok(("", Stmt::If(
            expr("true").unwrap().1,
            block("{a::b;}").unwrap().1,
        ))));

        assert_eq!(stmt("if(\"a\" == \"a\"){ a::b; }"), Ok(("", Stmt::If(
            expr("\"a\" == \"a\"").unwrap().1,
            block("{a::b;}").unwrap().1,
        ))));
        assert_eq!(stmt("if(foo() == \"a\"){ a::b; }"), Ok(("", Stmt::If(
            Expr::Eq(
                Box::new(Expr::FunctionCall("foo".to_string(), vec![])),
                Box::new(Expr::String("a".to_string())),
            ),
            block("{a::b;}").unwrap().1,
        ))));
    }

    #[test]
    fn test_operator_equal() {
        assert_eq!(expr("true == true"), Ok(("", Expr::Eq(
            Box::new(Expr::Boolean(true)),
            Box::new(Expr::Boolean(true)),
        ))));
        assert_eq!(expr("\"hello world\" == \"hello world\""), Ok(("", Expr::Eq(
            Box::new(Expr::String("hello world".to_string())),
            Box::new(Expr::String("hello world".to_string())),
        ))));
        assert_eq!(expr("\"22hello\" == true"), Ok(("", Expr::Eq(
            Box::new(Expr::String("22hello".to_string())),
            Box::new(Expr::Boolean(true)),
        ))));
    }

    #[test]
    fn test_key() {
        assert_eq!(key("a"), Ok(("", ParsedSingleKey::Key(*KEY_A))));
        // assert_eq!(key("mouse5"), Ok(("", ParsedSingleKey::Key(*KEY_MOUSE5))));
        assert_eq!(key("A"), Ok(("", ParsedSingleKey::CapitalKey(*KEY_A))));
        assert_eq!(key("enter"), Ok(("", ParsedSingleKey::Key(*KEY_ENTER))));
        assert!(matches!(key("entert"), Err(..)));
    }

    #[test]
    fn test_key_action() {
        assert_eq!(key_action_with_flags("!a"), Ok(("", ParsedKeyAction::KeyClickAction(
            KeyClickActionWithMods::new_with_mods(
                *KEY_A,
                KeyModifierFlags::new().tap_mut(|v|v.alt()),
            )))));

        // assert_eq!(key_action("!#^a"), Ok(("", ParsedKeyAction::KeyClickAction(
        //     KeyClickActionWithMods::new_with_mods(
        //         *KEY_A,
        //         *KeyModifierFlags::new().ctrl().alt().meta(),
        //     )))));
        //
        // assert_eq!(key_action("A"), Ok(("", ParsedKeyAction::KeyClickAction(
        //     KeyClickActionWithMods::new_with_mods(
        //         *KEY_A,
        //         *KeyModifierFlags::new().shift(),
        //     )))));
        //
        // assert_eq!(key_action("+A"), Ok(("", ParsedKeyAction::KeyClickAction(
        //     KeyClickActionWithMods::new_with_mods(
        //         *KEY_A,
        //         *KeyModifierFlags::new().shift(),
        //     )))));
        //
        // assert!(matches!(key_action("+al"), Err(..)));
        //
        // assert!(matches!(key_action("++a"), Err(..)));
    }

    #[test]
    fn test_block() {
        assert_eq!(block_body("a::b;"), Ok(("", Block::new()
            .tap_mut(|b| { b.statements.push(stmt("a::b;").unwrap().1); })
        )));

        assert_eq!(block("{ let foo = true; }"), Ok(("", Block::new()
            .tap_mut(|b| { b.statements.push(stmt("let foo = true;").unwrap().1); })
        )));

        assert_eq!(block("{ let foo = true; a::b; b::c; }"), Ok(("", Block::new()
            .tap_mut(|b| {
                b.statements.push(stmt("let foo = true;").unwrap().1);
                b.statements.push(stmt("a::b;").unwrap().1);
                b.statements.push(stmt("b::c;").unwrap().1);
            })
        )));

        assert_eq!(block_body("if(true){a::b;}"), Ok(("", Block::new().tap_mut(|b| {
            b.statements = vec![
                if_stmt("if(true){a::b;}").unwrap().1
            ];
        }))));
    }
}