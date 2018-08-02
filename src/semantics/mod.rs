//! The semantics of the language
//!
//! Here we define the rules of normalization, type checking, and type inference.
//!
//! For more information, check out the theory appendix of the DDL book.

use codespan::ByteSpan;
use im::HashMap;
use moniker::{Binder, BoundTerm, Embed, FreeVar, Nest, Scope, Var};
use num_traits::ToPrimitive;

use syntax::core::{
    Definition, Head, Literal, Module, Neutral, Pattern, RcNeutral, RcPattern, RcTerm, RcType,
    RcValue, Term, Type, Value,
};
use syntax::raw;
use syntax::translation::Resugar;
use syntax::Level;

mod errors;
pub mod parser;
mod prim;
#[cfg(test)]
mod tests;

pub use self::errors::{InternalError, TypeError};
pub use self::prim::{PrimEnv, PrimFn};

/// The type checking environment
///
/// A default environment with entries for built-in types is provided via the
/// implementation of the `Default` trait.
///
/// We use persistent data structures internally so that we can copy the
/// environment as we enter into scopes, without having to deal with the
/// error-prone tedium of working with mutable context.
#[derive(Clone, Debug)]
pub struct TcEnv {
    /// Primitive definitions
    pub primitives: PrimEnv,
    /// Global annotation/definition pairs
    pub globals: HashMap<&'static str, (Option<RcValue>, RcType)>,
    /// The type annotations of the binders we have passed over
    pub claims: HashMap<FreeVar<String>, RcType>,
    /// Any definitions we have passed over
    pub definitions: HashMap<FreeVar<String>, RcTerm>,
}

impl Default for TcEnv {
    fn default() -> TcEnv {
        use num_bigint::BigInt;

        fn int_ty<T: Into<BigInt>>(min: Option<T>, max: Option<T>) -> RcValue {
            RcValue::from(Value::IntType(
                min.map(|x| RcValue::from(Value::Literal(Literal::Int(x.into())))),
                max.map(|x| RcValue::from(Value::Literal(Literal::Int(x.into())))),
            ))
        }

        let universe0 = RcValue::from(Value::universe(0));
        let true_value = RcValue::from(Value::Literal(Literal::Bool(true)));
        let false_value = RcValue::from(Value::Literal(Literal::Bool(false)));
        let bool_ty = RcValue::from(Value::global("Bool"));
        let nat_ty = RcValue::from(Value::IntType(
            Some(RcValue::from(Value::Literal(Literal::Int(0.into())))),
            None,
        ));
        let arrow = |params: Vec<RcType>, ret: RcType| {
            params.into_iter().rev().fold(ret, |body, ann| {
                RcValue::from(Value::Pi(Scope::new(
                    (Binder(FreeVar::fresh_unnamed()), Embed(ann)),
                    body,
                )))
            })
        };

        TcEnv {
            primitives: PrimEnv::default(),
            globals: hashmap!{
                "Bool" => (None, universe0.clone()),
                "true" => (Some(true_value), bool_ty.clone()),
                "false" => (Some(false_value), bool_ty.clone()),
                "String" => (None, universe0.clone()),
                "Char" => (None, universe0.clone()),

                "U8" => (Some(int_ty(Some(u8::min_value()), Some(u8::max_value()))), universe0.clone()),
                "U16" => (Some(int_ty(Some(u16::min_value()), Some(u16::max_value()))), universe0.clone()),
                "U32" => (Some(int_ty(Some(u32::min_value()), Some(u32::max_value()))), universe0.clone()),
                "U64" => (Some(int_ty(Some(u64::min_value()), Some(u64::max_value()))), universe0.clone()),
                "S8" => (Some(int_ty(Some(i8::min_value()), Some(i8::max_value()))), universe0.clone()),
                "S16" => (Some(int_ty(Some(i16::min_value()), Some(i16::max_value()))), universe0.clone()),
                "S32" => (Some(int_ty(Some(i32::min_value()), Some(i32::max_value()))), universe0.clone()),
                "S64" => (Some(int_ty(Some(i64::min_value()), Some(i64::max_value()))), universe0.clone()),

                "F32" => (None, universe0.clone()),
                "F64" => (None, universe0.clone()),
                "Array" => (None, arrow(vec![nat_ty, universe0.clone()], universe0.clone())),

                // TODO: Replace these with more general compute types
                "U16Le" => (None, universe0.clone()),
                "U32Le" => (None, universe0.clone()),
                "U64Le" => (None, universe0.clone()),
                "S16Le" => (None, universe0.clone()),
                "S32Le" => (None, universe0.clone()),
                "S64Le" => (None, universe0.clone()),
                "F32Le" => (None, universe0.clone()),
                "F64Le" => (None, universe0.clone()),
                "U16Be" => (None, universe0.clone()),
                "U32Be" => (None, universe0.clone()),
                "U64Be" => (None, universe0.clone()),
                "S16Be" => (None, universe0.clone()),
                "S32Be" => (None, universe0.clone()),
                "S64Be" => (None, universe0.clone()),
                "F32Be" => (None, universe0.clone()),
                "F64Be" => (None, universe0.clone()),
            },
            claims: hashmap!{},
            definitions: hashmap!{},
        }
    }
}

/// Type check and elaborate a module
pub fn check_module(raw_module: &raw::Module) -> Result<Module, TypeError> {
    let mut tc_env = TcEnv::default();
    let definitions = raw_module
        .definitions
        .clone()
        .unnest()
        .into_iter()
        .map(|(Binder(free_var), Embed(raw_definition))| {
            let (term, ann) = match *raw_definition.ann.inner {
                // We don't have a type annotation available to us! Instead we will
                // attempt to infer it based on the body of the definition
                raw::Term::Hole(_) => infer_term(&tc_env, &raw_definition.term)?,
                // We have a type annotation! Elaborate it, then normalize it, then
                // check that it matches the body of the definition
                _ => {
                    let (ann, _) = infer_term(&tc_env, &raw_definition.ann)?;
                    let ann = normalize(&tc_env, &ann)?;
                    let term = check_term(&tc_env, &raw_definition.term, &ann)?;
                    (term, ann)
                },
            };

            // Add the definition to the type checking environment
            tc_env.claims.insert(free_var.clone(), ann.clone());
            tc_env.definitions.insert(free_var.clone(), term.clone());

            Ok((Binder(free_var), Embed(Definition { term, ann })))
        }).collect::<Result<_, TypeError>>()?;

    Ok(Module {
        name: raw_module.name.clone(),
        definitions: Nest::new(definitions),
    })
}

/// Reduce a term to its normal form
pub fn normalize(tc_env: &TcEnv, term: &RcTerm) -> Result<RcValue, InternalError> {
    match *term.inner {
        // E-ANN
        Term::Ann(ref expr, _) => normalize(tc_env, expr),

        // E-TYPE
        Term::Universe(level) => Ok(RcValue::from(Value::Universe(level))),

        Term::IntType(ref min, ref max) => {
            let min = match *min {
                None => None,
                Some(ref x) => Some(normalize(tc_env, x)?),
            };

            let max = match *max {
                None => None,
                Some(ref x) => Some(normalize(tc_env, x)?),
            };

            Ok(RcValue::from(Value::IntType(min, max)))
        },

        Term::Literal(ref lit) => Ok(RcValue::from(Value::Literal(lit.clone()))),

        // E-VAR, E-VAR-DEF
        Term::Var(ref var) => match *var {
            Var::Free(ref name) => match tc_env.definitions.get(name) {
                Some(term) => normalize(tc_env, term),
                None => Ok(RcValue::from(Value::from(var.clone()))),
            },

            // We should always be substituting bound variables with fresh
            // variables when entering scopes using `unbind`, so if we've
            // encountered one here this is definitely a bug!
            Var::Bound(_) => Err(InternalError::UnsubstitutedDebruijnIndex {
                span: None,
                var: var.clone(),
            }),
        },

        Term::Extern(ref name, ref ty) => Ok(RcValue::from(Value::from(Neutral::Head(
            Head::Extern(name.clone(), normalize(tc_env, ty)?),
        )))),

        Term::Global(ref name) => match tc_env.globals.get(name.as_str()) {
            Some(&(Some(ref value), _)) => Ok(value.clone()),
            Some(&(None, _)) | None => Ok(RcValue::from(Value::global(name.clone()))),
        },

        // E-PI
        Term::Pi(ref scope) => {
            let ((name, Embed(ann)), body) = scope.clone().unbind();

            Ok(RcValue::from(Value::Pi(Scope::new(
                (name, Embed(normalize(tc_env, &ann)?)),
                normalize(tc_env, &body)?,
            ))))
        },

        // E-LAM
        Term::Lam(ref scope) => {
            let ((name, Embed(ann)), body) = scope.clone().unbind();

            Ok(RcValue::from(Value::Lam(Scope::new(
                (name, Embed(normalize(tc_env, &ann)?)),
                normalize(tc_env, &body)?,
            ))))
        },

        // E-APP
        Term::App(ref head, ref arg) => {
            match *normalize(tc_env, head)?.inner {
                Value::Lam(ref scope) => {
                    // FIXME: do a local unbind here
                    let ((Binder(free_var), Embed(_)), body) = scope.clone().unbind();
                    normalize(tc_env, &body.substs(&[(free_var, arg.clone())]))
                },
                Value::Neutral(ref neutral, ref spine) => {
                    let arg = normalize(tc_env, arg)?;
                    let mut spine = spine.clone();

                    match *neutral.inner {
                        Neutral::Head(Head::Extern(ref name, _)) => {
                            spine.push_back(arg);

                            // Apply the arguments to primitive definitions if the number of
                            // arguments matches the arity of the primitive, all aof the arguments
                            // are fully normalized
                            if let Some(prim) = tc_env.primitives.get(name) {
                                if prim.arity == spine.len() && spine.iter().all(|arg| arg.is_nf())
                                {
                                    match (prim.interpretation)(spine) {
                                        Ok(value) => return Ok(value),
                                        Err(()) => unimplemented!("proper error"),
                                    }
                                }
                            }
                        },
                        Neutral::Head(Head::Var(_))
                        | Neutral::Head(Head::Global(_))
                        | Neutral::If(_, _, _)
                        | Neutral::Proj(_, _)
                        | Neutral::Case(_, _) => spine.push_back(arg),
                    }

                    Ok(RcValue::from(Value::Neutral(neutral.clone(), spine)))
                },
                _ => Err(InternalError::ArgumentAppliedToNonFunction),
            }
        },

        // E-IF, E-IF-TRUE, E-IF-FALSE
        Term::If(ref cond, ref if_true, ref if_false) => {
            let value_cond = normalize(tc_env, cond)?;

            match *value_cond {
                Value::Literal(Literal::Bool(true)) => normalize(tc_env, if_true),
                Value::Literal(Literal::Bool(false)) => normalize(tc_env, if_false),
                Value::Neutral(ref cond, ref spine) => Ok(RcValue::from(Value::Neutral(
                    RcNeutral::from(Neutral::If(
                        cond.clone(),
                        normalize(tc_env, if_true)?,
                        normalize(tc_env, if_false)?,
                    )),
                    spine.clone(),
                ))),
                _ => Err(InternalError::ExpectedBoolExpr),
            }
        },

        // E-RECORD-TYPE
        Term::RecordType(ref scope) => {
            let ((label, binder, Embed(ann)), body) = scope.clone().unbind();
            let ann = normalize(tc_env, &ann)?;
            let body = normalize(tc_env, &body)?;

            Ok(Value::RecordType(Scope::new((label, binder, Embed(ann)), body)).into())
        },

        // E-EMPTY-RECORD-TYPE
        Term::RecordTypeEmpty => Ok(RcValue::from(Value::RecordTypeEmpty)),

        // E-RECORD
        Term::Record(ref scope) => {
            let ((label, binder, Embed(term)), body) = scope.clone().unbind();
            let value = normalize(tc_env, &term)?;
            let body = normalize(tc_env, &body)?;

            Ok(Value::Record(Scope::new((label, binder, Embed(value)), body)).into())
        },

        // E-EMPTY-RECORD
        Term::RecordEmpty => Ok(RcValue::from(Value::RecordEmpty)),

        // E-PROJ
        Term::Proj(ref expr, ref label) => match *normalize(tc_env, expr)? {
            Value::Neutral(ref neutral, ref spine) => Ok(RcValue::from(Value::Neutral(
                RcNeutral::from(Neutral::Proj(neutral.clone(), label.clone())),
                spine.clone(),
            ))),
            ref expr => match expr.lookup_record(label) {
                Some(value) => Ok(value.clone()),
                None => Err(InternalError::ProjectedOnNonExistentField {
                    label: label.clone(),
                }),
            },
        },

        // E-CASE
        Term::Case(ref head, ref clauses) => {
            let head = normalize(tc_env, head)?;

            if let Value::Neutral(ref neutral, ref spine) = *head {
                Ok(RcValue::from(Value::Neutral(
                    RcNeutral::from(Neutral::Case(
                        neutral.clone(),
                        clauses
                            .iter()
                            .map(|clause| {
                                let (pattern, body) = clause.clone().unbind();
                                Ok(Scope::new(pattern, normalize(tc_env, &body)?))
                            }).collect::<Result<_, _>>()?,
                    )),
                    spine.clone(),
                )))
            } else {
                for clause in clauses {
                    let (pattern, body) = clause.clone().unbind();
                    if let Some(mappings) = match_value(&pattern, &head) {
                        let mappings = mappings
                            .into_iter()
                            .map(|(free_var, value)| (free_var, RcTerm::from(&*value.inner)))
                            .collect::<Vec<_>>();
                        return normalize(tc_env, &body.substs(&mappings));
                    }
                }
                Err(InternalError::NoPatternsApplicable)
            }
        },

        // E-ARRAY
        Term::Array(ref elems) => Ok(RcValue::from(Value::Array(
            elems
                .iter()
                .map(|elem| normalize(tc_env, elem))
                .collect::<Result<_, _>>()?,
        ))),
    }
}

/// If the pattern matches the value, this function returns the substitutions
/// needed to apply the pattern to some body expression
pub fn match_value(
    pattern: &RcPattern,
    value: &RcValue,
) -> Option<Vec<(FreeVar<String>, RcValue)>> {
    match (&*pattern.inner, &*value.inner) {
        (&Pattern::Literal(ref pattern_lit), &Value::Literal(ref value_lit))
            if pattern_lit == value_lit =>
        {
            Some(vec![])
        },
        (&Pattern::Binder(Binder(ref free_var)), _) => {
            Some(vec![(free_var.clone(), value.clone())])
        },
        (_, _) => None,
    }
}

/// Check that `ty1` is a subtype of `ty2`
pub fn is_subtype(ty1: &RcType, ty2: &RcType) -> bool {
    use num_bigint::BigInt;
    use std::{i16, i32, i64, u16, u32, u64};

    fn is_name(ty: &Type, name: &str) -> bool {
        if let Value::Neutral(ref neutral, ref spine) = *ty {
            if let Neutral::Head(Head::Global(ref n)) = **neutral {
                return name == *n && spine.is_empty();
            }
        }
        false
    }

    fn int_ty<T: Into<BigInt>>(min: Option<T>, max: Option<T>) -> RcValue {
        RcValue::from(Value::IntType(
            min.map(|x| RcValue::from(Value::Literal(Literal::Int(x.into())))),
            max.map(|x| RcValue::from(Value::Literal(Literal::Int(x.into())))),
        ))
    }

    match (&*ty1.inner, &*ty2.inner) {
        (&Value::IntType(ref min1, ref max1), &Value::IntType(ref min2, ref max2)) => {
            let in_min_bound = match (min1, min2) {
                (None, None) => true,     // -∞ <= -∞
                (Some(_), None) => true,  //  n <= -∞
                (None, Some(_)) => false, // -∞ <=  n
                (Some(ref min1), Some(ref min2)) => match (&*min1.inner, &*min2.inner) {
                    (
                        Value::Literal(Literal::Int(ref min1)),
                        Value::Literal(Literal::Int(ref min2)),
                    ) => min1 >= min2,
                    _ => Value::term_eq(min1, min2), // Fallback to alpha-equality
                },
            };

            let in_max_bound = match (max1, max2) {
                (None, None) => true,     // +∞ <= +∞
                (Some(_), None) => true,  //  n <= +∞
                (None, Some(_)) => false, // +∞ <=  n
                (Some(ref max1), Some(ref max2)) => match (&*max1.inner, &*max2.inner) {
                    (
                        Value::Literal(Literal::Int(ref max1)),
                        Value::Literal(Literal::Int(ref max2)),
                    ) => max1 <= max2,
                    _ => Value::term_eq(max1, max2), // Fallback to alpha-equality
                },
            };

            in_min_bound && in_max_bound
        },

        (t1, _) if is_name(t1, "U16Le") => is_subtype(&int_ty(Some(u16::MIN), Some(u16::MAX)), ty2),
        (t1, _) if is_name(t1, "U32Le") => is_subtype(&int_ty(Some(u32::MIN), Some(u32::MAX)), ty2),
        (t1, _) if is_name(t1, "U64Le") => is_subtype(&int_ty(Some(u64::MIN), Some(u64::MAX)), ty2),
        (t1, _) if is_name(t1, "S16Le") => is_subtype(&int_ty(Some(i16::MIN), Some(i16::MAX)), ty2),
        (t1, _) if is_name(t1, "S32Le") => is_subtype(&int_ty(Some(i32::MIN), Some(i32::MAX)), ty2),
        (t1, _) if is_name(t1, "S64Le") => is_subtype(&int_ty(Some(i64::MIN), Some(i64::MAX)), ty2),
        (t1, t2) if is_name(t1, "F32Le") && is_name(t2, "F32") => true,
        (t1, t2) if is_name(t1, "F64Le") && is_name(t2, "F64") => true,
        (t1, _) if is_name(t1, "U16Be") => is_subtype(&int_ty(Some(u16::MIN), Some(u16::MAX)), ty2),
        (t1, _) if is_name(t1, "U32Be") => is_subtype(&int_ty(Some(u32::MIN), Some(u32::MAX)), ty2),
        (t1, _) if is_name(t1, "U64Be") => is_subtype(&int_ty(Some(u64::MIN), Some(u64::MAX)), ty2),
        (t1, _) if is_name(t1, "S16Be") => is_subtype(&int_ty(Some(i16::MIN), Some(i16::MAX)), ty2),
        (t1, _) if is_name(t1, "S32Be") => is_subtype(&int_ty(Some(i32::MIN), Some(i32::MAX)), ty2),
        (t1, _) if is_name(t1, "S64Be") => is_subtype(&int_ty(Some(i64::MIN), Some(i64::MAX)), ty2),
        (t1, t2) if is_name(t1, "F32Be") && is_name(t2, "F32") => true,
        (t1, t2) if is_name(t1, "F64Be") && is_name(t2, "F64") => true,

        // Fallback to alpha-equality
        _ => Type::term_eq(ty1, ty2),
    }
}

/// Ensures that the given term is a universe, returning the level of that
/// universe and its elaborated form.
fn infer_universe(tc_env: &TcEnv, raw_term: &raw::RcTerm) -> Result<(RcTerm, Level), TypeError> {
    let (term, ty) = infer_term(tc_env, raw_term)?;
    match *ty {
        Value::Universe(level) => Ok((term, level)),
        _ => Err(TypeError::ExpectedUniverse {
            span: raw_term.span(),
            found: Box::new(ty.resugar()),
        }),
    }
}

/// Checks that a literal is compatible with the given type, returning the
/// elaborated literal if successful
fn check_literal(raw_literal: &raw::Literal, expected_ty: &RcType) -> Result<Literal, TypeError> {
    match expected_ty.global_app() {
        Some((name, spine)) if spine.is_empty() => {
            match (raw_literal, name) {
                (&raw::Literal::String(_, ref val), "String") => {
                    return Ok(Literal::String(val.clone()));
                },
                (&raw::Literal::Char(_, val), "Char") => return Ok(Literal::Char(val)),
                // FIXME: overflow?
                (&raw::Literal::Int(_, ref val), "F32") => {
                    return Ok(Literal::F32(val.to_f32().unwrap()))
                },
                (&raw::Literal::Int(_, ref val), "F64") => {
                    return Ok(Literal::F64(val.to_f64().unwrap()))
                },
                (&raw::Literal::Float(_, val), "F32") => return Ok(Literal::F32(val as f32)),
                (&raw::Literal::Float(_, val), "F64") => return Ok(Literal::F64(val)),

                _ => {},
            }
        },
        Some(_) | None => {},
    }

    let (literal, inferred_ty) = infer_literal(raw_literal)?;
    if is_subtype(&inferred_ty, expected_ty) {
        Ok(literal)
    } else {
        Err(TypeError::LiteralMismatch {
            literal_span: raw_literal.span(),
            found: raw_literal.clone(),
            expected: Box::new(expected_ty.resugar()),
        })
    }
}

/// Synthesize the type of a literal, returning the elaborated literal and the
/// inferred type if successful
fn infer_literal(raw_literal: &raw::Literal) -> Result<(Literal, RcType), TypeError> {
    match *raw_literal {
        raw::Literal::String(_, ref value) => Ok((
            Literal::String(value.clone()),
            RcValue::from(Value::global("String")),
        )),
        raw::Literal::Char(_, value) => {
            Ok((Literal::Char(value), RcValue::from(Value::global("Char"))))
        },
        raw::Literal::Int(_, ref value) => Ok((Literal::Int(value.clone()), {
            let value = RcValue::from(Value::Literal(Literal::Int(value.clone())));
            RcValue::from(Value::IntType(Some(value.clone()), Some(value)))
        })),
        raw::Literal::Float(span, _) => Err(TypeError::AmbiguousFloatLiteral { span }),
    }
}

/// Checks that a pattern is compatible with the given type, returning the
/// elaborated pattern and a vector of the claims it introduced if successful
pub fn check_pattern(
    tc_env: &TcEnv,
    raw_pattern: &raw::RcPattern,
    expected_ty: &RcType,
) -> Result<(RcPattern, Vec<(FreeVar<String>, RcType)>), TypeError> {
    match (&*raw_pattern.inner, &*expected_ty.inner) {
        (&raw::Pattern::Binder(_, Binder(ref free_var)), _) => {
            return Ok((
                RcPattern::from(Pattern::Binder(Binder(free_var.clone()))),
                vec![(free_var.clone(), expected_ty.clone())],
            ));
        },
        (&raw::Pattern::Literal(ref raw_literal), _) => {
            let literal = check_literal(raw_literal, expected_ty)?;
            return Ok((RcPattern::from(Pattern::Literal(literal)), vec![]));
        },
        _ => {},
    }

    let (pattern, inferred_ty, claims) = infer_pattern(tc_env, raw_pattern)?;
    if Type::term_eq(&inferred_ty, expected_ty) {
        Ok((pattern, claims))
    } else {
        Err(TypeError::Mismatch {
            span: raw_pattern.span(),
            found: Box::new(inferred_ty.resugar()),
            expected: Box::new(expected_ty.resugar()),
        })
    }
}

/// Synthesize the type of a pattern, returning the elaborated pattern, the
/// inferred type, and a vector of the claims it introduced if successful
pub fn infer_pattern(
    tc_env: &TcEnv,
    raw_pattern: &raw::RcPattern,
) -> Result<(RcPattern, RcType, Vec<(FreeVar<String>, RcType)>), TypeError> {
    match *raw_pattern.inner {
        raw::Pattern::Ann(ref raw_pattern, Embed(ref raw_ty)) => {
            let (ty, _) = infer_universe(tc_env, raw_ty)?;
            let value_ty = normalize(tc_env, &ty)?;
            let (pattern, claims) = check_pattern(tc_env, raw_pattern, &value_ty)?;

            Ok((
                RcPattern::from(Pattern::Ann(pattern, Embed(ty))),
                value_ty,
                claims,
            ))
        },
        raw::Pattern::Literal(ref literal) => {
            let (literal, ty) = infer_literal(literal)?;
            Ok((RcPattern::from(Pattern::Literal(literal)), ty, vec![]))
        },
        raw::Pattern::Binder(span, ref binder) => Err(TypeError::BinderNeedsAnnotation {
            span,
            binder: binder.clone(),
        }),
    }
}

/// Checks that a term is compatible with the given type, returning the
/// elaborated term if successful
pub fn check_term(
    tc_env: &TcEnv,
    raw_term: &raw::RcTerm,
    expected_ty: &RcType,
) -> Result<RcTerm, TypeError> {
    match (&*raw_term.inner, &*expected_ty.inner) {
        (&raw::Term::Literal(ref raw_literal), _) => {
            let literal = check_literal(raw_literal, expected_ty)?;
            return Ok(RcTerm::from(Term::Literal(literal)));
        },

        // C-LAM
        (&raw::Term::Lam(_, ref lam_scope), &Value::Pi(ref pi_scope)) => {
            let ((lam_name, Embed(lam_ann)), lam_body, (Binder(pi_name), Embed(pi_ann)), pi_body) =
                Scope::unbind2(lam_scope.clone(), pi_scope.clone());

            // Elaborate the hole, if it exists
            if let raw::Term::Hole(_) = *lam_ann.inner {
                let lam_ann = RcTerm::from(Term::from(&*pi_ann));
                let lam_body = {
                    let mut body_tc_env = tc_env.clone();
                    body_tc_env.claims.insert(pi_name, pi_ann);
                    check_term(&body_tc_env, &lam_body, &pi_body)?
                };
                let lam_scope = Scope::new((lam_name, Embed(lam_ann)), lam_body);

                return Ok(RcTerm::from(Term::Lam(lam_scope)));
            }

            // TODO: We might want to optimise for this case, rather than
            // falling through to `infer` and unbinding again at I-LAM
        },
        (&raw::Term::Lam(_, _), _) => {
            return Err(TypeError::UnexpectedFunction {
                span: raw_term.span(),
                expected: Box::new(expected_ty.resugar()),
            });
        },

        // C-IF
        (&raw::Term::If(_, ref raw_cond, ref raw_if_true, ref raw_if_false), _) => {
            let bool_ty = RcValue::from(Value::global("Bool"));
            let cond = check_term(tc_env, raw_cond, &bool_ty)?;
            let if_true = check_term(tc_env, raw_if_true, expected_ty)?;
            let if_false = check_term(tc_env, raw_if_false, expected_ty)?;

            return Ok(RcTerm::from(Term::If(cond, if_true, if_false)));
        },

        // C-RECORD
        (&raw::Term::Record(span, ref scope), &Value::RecordType(ref ty_scope)) => {
            let (
                (label, binder, Embed(raw_expr)),
                raw_body,
                (ty_label, ty_binder, Embed(ann)),
                ty_body,
            ) = Scope::unbind2(scope.clone(), ty_scope.clone());

            if label == ty_label {
                let expr = check_term(tc_env, &raw_expr, &ann)?;
                let ty_body = normalize(tc_env, &ty_body.substs(&[(ty_binder.0, expr.clone())]))?;
                let body = check_term(tc_env, &raw_body, &ty_body)?;

                return Ok(RcTerm::from(Term::Record(Scope::new(
                    (label, binder, Embed(expr)),
                    body,
                ))));
            } else {
                return Err(TypeError::LabelMismatch {
                    span,
                    found: label,
                    expected: ty_label,
                });
            }
        },

        (&raw::Term::Case(_, ref raw_head, ref raw_clauses), _) => {
            let (head, head_ty) = infer_term(tc_env, raw_head)?;

            // TODO: ensure that patterns are exhaustive
            let clauses = raw_clauses
                .iter()
                .map(|raw_clause| {
                    let (raw_pattern, raw_body) = raw_clause.clone().unbind();
                    let (pattern, claims) = check_pattern(tc_env, &raw_pattern, &head_ty)?;

                    let mut body_tc_env = tc_env.clone();
                    body_tc_env.claims.extend(claims);
                    let body = check_term(&body_tc_env, &raw_body, expected_ty)?;

                    Ok(Scope::new(pattern, body))
                }).collect::<Result<_, TypeError>>()?;

            return Ok(RcTerm::from(Term::Case(head, clauses)));
        },

        (&raw::Term::Array(span, ref elems), ty) => match ty.global_app() {
            Some(("Array", spine)) if spine.len() == 2 => {
                let len = &spine[0];
                let elem_ty = &spine[1];
                if let Value::Literal(Literal::Int(ref len)) = **len {
                    if *len != elems.len().into() {
                        return Err(TypeError::ArrayLengthMismatch {
                            span,
                            found_len: elems.len(),
                            expected_len: len.clone(),
                        });
                    }
                }

                return Ok(RcTerm::from(Term::Array(
                    elems
                        .iter()
                        .map(|elem| check_term(tc_env, elem, elem_ty))
                        .collect::<Result<_, _>>()?,
                )));
            },
            Some(_) | None => unimplemented!(),
        },

        (&raw::Term::Hole(span), _) => {
            let expected = Some(Box::new(expected_ty.resugar()));
            return Err(TypeError::UnableToElaborateHole { span, expected });
        },

        _ => {},
    }

    // C-CONV
    let (term, inferred_ty) = infer_term(tc_env, raw_term)?;
    if is_subtype(&inferred_ty, expected_ty) {
        Ok(term)
    } else {
        Err(TypeError::Mismatch {
            span: raw_term.span(),
            found: Box::new(inferred_ty.resugar()),
            expected: Box::new(expected_ty.resugar()),
        })
    }
}

/// Synthesize the type of a term, returning the elaborated term and the
/// inferred type if successful
pub fn infer_term(tc_env: &TcEnv, raw_term: &raw::RcTerm) -> Result<(RcTerm, RcType), TypeError> {
    use std::cmp;

    match *raw_term.inner {
        //  I-ANN
        raw::Term::Ann(ref raw_expr, ref raw_ty) => {
            let (ty, _) = infer_universe(tc_env, raw_ty)?;
            let value_ty = normalize(tc_env, &ty)?;
            let expr = check_term(tc_env, raw_expr, &value_ty)?;

            Ok((RcTerm::from(Term::Ann(expr, ty)), value_ty))
        },

        // I-TYPE
        raw::Term::Universe(_, level) => Ok((
            RcTerm::from(Term::Universe(level)),
            RcValue::from(Value::Universe(level.succ())),
        )),

        raw::Term::Hole(span) => {
            let expected = None;
            Err(TypeError::UnableToElaborateHole { span, expected })
        },

        raw::Term::IntType(_, ref min, ref max) => {
            let min = match *min {
                None => None,
                Some(ref min) => {
                    let any_int = RcValue::from(Value::IntType(None, None));
                    Some(check_term(tc_env, min, &any_int)?)
                },
            };

            let max = match *max {
                None => None,
                Some(ref max) => {
                    let any_int = RcValue::from(Value::IntType(None, None));
                    Some(check_term(tc_env, max, &any_int)?)
                },
            };

            Ok((
                RcTerm::from(Term::IntType(min, max)),
                RcValue::from(Value::Universe(Level(0))),
            ))
        },

        raw::Term::Literal(ref raw_literal) => {
            let (literal, ty) = infer_literal(raw_literal)?;
            Ok((RcTerm::from(Term::Literal(literal)), ty))
        },

        // I-VAR
        raw::Term::Var(span, ref var) => match *var {
            Var::Free(ref free_var) => match tc_env.claims.get(free_var) {
                Some(ty) => Ok((RcTerm::from(Term::Var(var.clone())), ty.clone())),
                None => Err(InternalError::UndefinedFreeVar {
                    span,
                    free_var: free_var.clone(),
                }.into()),
            },

            // We should always be substituting bound variables with fresh
            // variables when entering scopes using `unbind`, so if we've
            // encountered one here this is definitely a bug!
            Var::Bound(_) => Err(InternalError::UnsubstitutedDebruijnIndex {
                span: Some(raw_term.span()),
                var: var.clone(),
            }.into()),
        },

        raw::Term::Extern(_, name_span, ref name, _) if tc_env.primitives.get(name).is_none() => {
            Err(TypeError::UndefinedExternName {
                span: name_span,
                name: name.clone(),
            })
        },

        raw::Term::Extern(_, _, ref name, ref raw_ty) => {
            let (ty, _) = infer_universe(tc_env, raw_ty)?;
            let value_ty = normalize(tc_env, &ty)?;
            Ok((RcTerm::from(Term::Extern(name.clone(), ty)), value_ty))
        },

        raw::Term::Global(span, ref name) => match tc_env.globals.get(name.as_str()) {
            Some((_, ref ty)) => Ok((RcTerm::from(Term::global(name.clone())), ty.clone())),
            None => Err(TypeError::UndefinedName {
                span,
                name: name.clone(),
            }),
        },

        // I-PI
        raw::Term::Pi(_, ref raw_scope) => {
            let ((Binder(free_var), Embed(raw_ann)), raw_body) = raw_scope.clone().unbind();

            let (ann, ann_level) = infer_universe(tc_env, &raw_ann)?;
            let (body, body_level) = {
                let ann = normalize(tc_env, &ann)?;
                let mut body_tc_env = tc_env.clone();
                body_tc_env.claims.insert(free_var.clone(), ann);
                infer_universe(&body_tc_env, &raw_body)?
            };

            Ok((
                RcTerm::from(Term::Pi(Scope::new((Binder(free_var), Embed(ann)), body))),
                RcValue::from(Value::Universe(cmp::max(ann_level, body_level))),
            ))
        },

        // I-LAM
        raw::Term::Lam(_, ref raw_scope) => {
            let ((Binder(free_var), Embed(raw_ann)), raw_body) = raw_scope.clone().unbind();

            // Check for holes before entering to ensure we get a nice error
            if let raw::Term::Hole(_) = *raw_ann {
                return Err(TypeError::FunctionParamNeedsAnnotation {
                    param_span: ByteSpan::default(), // TODO: param.span(),
                    var_span: None,
                    name: free_var.clone(),
                });
            }

            let (lam_ann, _) = infer_universe(tc_env, &raw_ann)?;
            let pi_ann = normalize(tc_env, &lam_ann)?;
            let (lam_body, pi_body) = {
                let mut body_tc_env = tc_env.clone();
                body_tc_env.claims.insert(free_var.clone(), pi_ann.clone());
                infer_term(&body_tc_env, &raw_body)?
            };

            let lam_param = (Binder(free_var.clone()), Embed(lam_ann));
            let pi_param = (Binder(free_var.clone()), Embed(pi_ann));

            Ok((
                RcTerm::from(Term::Lam(Scope::new(lam_param, lam_body))),
                RcValue::from(Value::Pi(Scope::new(pi_param, pi_body))),
            ))
        },

        // I-IF
        raw::Term::If(_, ref raw_cond, ref raw_if_true, ref raw_if_false) => {
            let bool_ty = RcValue::from(Value::global("Bool"));
            let cond = check_term(tc_env, raw_cond, &bool_ty)?;
            let (if_true, ty) = infer_term(tc_env, raw_if_true)?;
            let if_false = check_term(tc_env, raw_if_false, &ty)?;

            Ok((RcTerm::from(Term::If(cond, if_true, if_false)), ty))
        },

        // I-APP
        raw::Term::App(ref raw_head, ref raw_arg) => {
            let (head, head_ty) = infer_term(tc_env, raw_head)?;

            match *head_ty {
                Value::Pi(ref scope) => {
                    let ((Binder(free_var), Embed(ann)), body) = scope.clone().unbind();

                    let arg = check_term(tc_env, raw_arg, &ann)?;
                    let body = normalize(tc_env, &body.substs(&[(free_var, arg.clone())]))?;

                    Ok((RcTerm::from(Term::App(head, arg)), body))
                },
                _ => Err(TypeError::ArgAppliedToNonFunction {
                    fn_span: raw_head.span(),
                    arg_span: raw_arg.span(),
                    found: Box::new(head_ty.resugar()),
                }),
            }
        },

        // I-RECORD-TYPE
        raw::Term::RecordType(_, ref raw_scope) => {
            let ((label, Binder(free_var), Embed(raw_ann)), raw_body) = raw_scope.clone().unbind();

            // Check that rest of record is well-formed?
            // Might be able to skip that for now, because there's no way to
            // express ill-formed records in the concrete syntax...

            let (ann, ann_level) = infer_universe(tc_env, &raw_ann)?;
            let (body, body_level) = {
                let ann = normalize(tc_env, &ann)?;
                let mut body_tc_env = tc_env.clone();
                body_tc_env.claims.insert(free_var.clone(), ann);
                infer_universe(&body_tc_env, &raw_body)?
            };

            let scope = Scope::new((label, Binder(free_var), Embed(ann)), body);

            Ok((
                RcTerm::from(Term::RecordType(scope)),
                RcValue::from(Value::Universe(cmp::max(ann_level, body_level))),
            ))
        },

        raw::Term::Record(span, _) => Err(TypeError::AmbiguousRecord { span }),

        // I-EMPTY-RECORD-TYPE
        raw::Term::RecordTypeEmpty(_) => Ok((
            RcTerm::from(Term::RecordTypeEmpty),
            RcValue::from(Value::universe(0)),
        )),

        // I-EMPTY-RECORD
        raw::Term::RecordEmpty(_) => Ok((
            RcTerm::from(Term::RecordEmpty),
            RcValue::from(Value::RecordTypeEmpty),
        )),

        // I-PROJ
        raw::Term::Proj(_, ref expr, label_span, ref label) => {
            let (expr, ty) = infer_term(tc_env, expr)?;

            let mut mappings = vec![];
            let mut current_scope = ty.record_ty();
            let mut field_ty = None;

            while let Some(scope) = current_scope {
                let ((current_label, current_binder, Embed(current_field_ty)), body) =
                    scope.unbind();

                if current_label == *label {
                    field_ty = Some(current_field_ty.substs(&mappings));
                    break;
                }

                let proj = RcTerm::from(Term::Proj(expr.clone(), current_label));
                mappings.push((current_binder.0, proj));
                current_scope = body.record_ty();
            }

            match field_ty {
                Some(field_ty) => Ok((
                    RcTerm::from(Term::Proj(expr, label.clone())),
                    normalize(tc_env, &field_ty)?,
                )),
                None => Err(TypeError::NoFieldInType {
                    label_span,
                    expected_label: label.clone(),
                    found: Box::new(ty.resugar()),
                }),
            }
        },

        // I-CASE
        raw::Term::Case(span, ref raw_head, ref raw_clauses) => {
            let (head, head_ty) = infer_term(tc_env, raw_head)?;
            let mut ty = None;

            // TODO: ensure that patterns are exhaustive
            let clauses = raw_clauses
                .iter()
                .map(|raw_clause| {
                    let (raw_pattern, raw_body) = raw_clause.clone().unbind();
                    let (pattern, claims) = check_pattern(tc_env, &raw_pattern, &head_ty)?;

                    let (body, body_ty) = {
                        let mut body_tc_env = tc_env.clone();
                        body_tc_env.claims.extend(claims);
                        infer_term(&body_tc_env, &raw_body)?
                    };

                    match ty {
                        None => ty = Some(body_ty),
                        Some(ref ty) if RcValue::term_eq(&body_ty, ty) => {},
                        Some(ref ty) => {
                            return Err(TypeError::Mismatch {
                                span: raw_body.span(),
                                found: Box::new(body_ty.resugar()),
                                expected: Box::new(ty.resugar()),
                            });
                        },
                    }

                    Ok(Scope::new(pattern, body))
                }).collect::<Result<_, TypeError>>()?;

            match ty {
                Some(ty) => Ok((RcTerm::from(Term::Case(head, clauses)), ty)),
                None => Err(TypeError::AmbiguousEmptyCase { span }),
            }
        },

        raw::Term::Array(span, _) => Err(TypeError::AmbiguousArrayLiteral { span }),
    }
}
