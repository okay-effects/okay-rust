//! `#[resumable(Ops)]`: a function whose body stops at `ask!(Effect::Op(args))`, made into an explicit REST — an enum,
//! one variant per stop, each holding the variables in scope there, `#[derive(Clone)]`; its steps
//! `okay::resumable::Step<Ops, Rest, A>`. The rest is a value: a handler resumes it as many times as it wants by
//! cloning it. No coroutine, no future, no replay: stable Rust.
//!
//! Each variant's resumption is straight code: from its stop to the next one, the rest of each block around it, a
//! loop's body finished and the loop entered again — no dispatch on a state inside a run.
//!
//! What it lowers: a stop as a statement (`let x: T = ask!(…);`, `ask!(…);`), `if`/`else`, `while`, `loop`, `match`
//! arms written as blocks and blocks holding stops, with `break`, `continue` and `return` inside; a statement holding
//! no stop is copied as it is. A variable living across a stop has a type written (`let x: T = …`): it is a field of
//! the rest. Not lowered (an error saying so): a stop inside an expression or a condition, a `for` holding a stop
//! (its iterator is not a field), labelled loops holding a stop, type parameters.
//!
//! ```compile_fail
//! # use okay::{ask, resumable::*};
//! # okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
//! # okay::ops! { pub enum Ops { Choose } }
//! #[okay_resumable::resumable(Ops)]
//! fn inside(n: usize) -> usize {
//!     1 + ask!(Choose::Among(n))     // a stop inside an expression: ask first, `let x: usize = ask!(…);`
//! }
//! ```
//!
//! ```compile_fail
//! # use okay::{ask, resumable::*};
//! # okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
//! # okay::ops! { pub enum Ops { Choose } }
//! #[okay_resumable::resumable(Ops)]
//! fn untyped(n: usize) -> usize {
//!     let k = n * 2;                  // lives across the stop: `let k: usize = …`
//!     let i: usize = ask!(Choose::Among(n));
//!     i + k
//! }
//! ```
//!
//! ```compile_fail
//! # use okay::{ask, resumable::*};
//! # okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
//! # okay::ops! { pub enum Ops { Choose } }
//! #[okay_resumable::resumable(Ops)]
//! fn each(n: usize) {
//!     for i in 0..n {                 // a `for` holding a stop: write it as a `while`
//!         let _: usize = ask!(Choose::Among(i));
//!     }
//! }
//! ```
//!
//! ```
//! # use okay::{ask, resumable::*};
//! # okay::effect! { pub enum Choose: Plain { Among(n: usize) -> usize = among; } }
//! # okay::ops! { pub enum Ops { Choose } }
//! #[okay_resumable::resumable(Ops)]
//! fn fine(n: usize) -> usize {
//!     let k: usize = n * 2;
//!     let i: usize = ask!(Choose::Among(n));
//!     i + k
//! }
//! let Step::Perform(Ops::Choose(Choose::Among(3, r)), k) = fine(3) else { panic!() };
//! let all: Vec<usize> = (0..3)
//!     .map(|i| match k.clone().resume(r.give(i)) { Step::Done(v) => v, _ => panic!() })
//!     .collect();
//! assert_eq!(all, [6, 7, 8]);
//! ```

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::visit::Visit;
use syn::visit_mut::VisitMut;
use syn::{Block, Expr, FnArg, Ident, ItemFn, Pat, ReturnType, Stmt, Type, parse_macro_input};

#[proc_macro_attribute]
pub fn resumable(attr: TokenStream, item: TokenStream) -> TokenStream {
    let ops: Type = parse_macro_input!(attr as Type);
    let f = parse_macro_input!(item as ItemFn);
    match generate(ops, f) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// is this an `ask!(…)`?
fn ask_mac(m: &syn::Macro) -> bool {
    m.path.segments.last().is_some_and(|s| s.ident == "ask")
}

/// any `ask!` inside (closures and nested items aside: theirs are their own)
fn has_stop_stmt(s: &Stmt) -> bool {
    struct Find(bool);
    impl<'a> Visit<'a> for Find {
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            if ask_mac(m) {
                self.0 = true;
            }
        }
        fn visit_expr_closure(&mut self, _: &'a syn::ExprClosure) {}
        fn visit_item(&mut self, _: &'a syn::Item) {}
    }
    let mut f = Find(false);
    f.visit_stmt(s);
    f.0
}
fn has_stop_expr(e: &Expr) -> bool {
    has_stop_stmt(&Stmt::Expr(e.clone(), None))
}

/// the `ask!` of a statement that is one, as the operation's path and arguments
fn ask_of(e: &Expr) -> Option<syn::Result<syn::ExprCall>> {
    let Expr::Macro(m) = e else { return None };
    if !ask_mac(&m.mac) {
        return None;
    }
    Some(m.mac.parse_body::<syn::ExprCall>())
}
fn ask_of_stmt(s: &Stmt) -> Option<syn::Result<syn::ExprCall>> {
    match s {
        Stmt::Expr(e, _) => ask_of(e),
        Stmt::Macro(m) if ask_mac(&m.mac) => Some(m.mac.parse_body::<syn::ExprCall>()),
        _ => None,
    }
}

/// the construct owning a block: a loop's body is re-entered after a stop in it; any other block is left
#[derive(Clone, Copy)]
enum Owner<'a> {
    Plain,
    Loop(&'a Expr, usize),
}

/// where a stop is: the blocks around it, outermost first — each the block's statements, the index of the one
/// leading to the stop, and what owns the block
#[derive(Clone)]
struct Frame<'a> {
    stmts: &'a [Stmt],
    index: usize,
    owner: Owner<'a>,
}

struct Stop<'a> {
    key: usize,
    call: syn::ExprCall,
    bind: Option<(Ident, Type)>,
    scope: Vec<(Ident, Type)>,
    frames: Vec<Frame<'a>>,
}

/// a variable declared: its name, its type if written
type Var = (Ident, Option<Type>);

struct Gen<'a> {
    name: Ident,
    ops: Type,
    stops: Vec<Stop<'a>>,
    loops: usize,
    errors: Vec<syn::Error>,
}

/// the stop statement this is, if it is one: the call, and the variable it binds
/// a stop: the operation's call, and the variable its answer is bound to
type Found = (syn::ExprCall, Option<(Ident, Type)>);

fn stop_of(s: &Stmt) -> Option<syn::Result<Found>> {
    if let Some(c) = ask_of_stmt(s) {
        return Some(c.map(|c| (c, None)));
    }
    let Stmt::Local(l) = s else { return None };
    let init = l.init.as_ref()?;
    let call = ask_of(&init.expr)?;
    Some(call.and_then(|c| match &l.pat {
        Pat::Wild(_) => Ok((c, None)),
        Pat::Type(pt) => match &*pt.pat {
            Pat::Wild(_) => Ok((c, None)),
            Pat::Ident(pi) => Ok((c, Some((pi.ident.clone(), (*pt.ty).clone())))),
            _ => Err(syn::Error::new_spanned(
                l,
                "a value asked is bound to a name: `let x: T = ask!(…);`",
            )),
        },
        _ => Err(syn::Error::new_spanned(
            l,
            "a value asked is bound with its type: `let x: T = ask!(…);`",
        )),
    }))
}

impl<'a> Gen<'a> {
    // THE STOPS FOUND: each with the variables in scope and the blocks around it
    fn find(
        &mut self,
        stmts: &'a [Stmt],
        scope: &mut Vec<Var>,
        frames: &mut Vec<Frame<'a>>,
        owner: Owner<'a>,
    ) {
        let depth = scope.len();
        for (i, s) in stmts.iter().enumerate() {
            frames.push(Frame {
                stmts,
                index: i,
                owner,
            });
            match stop_of(s) {
                Some(Ok((call, bind))) => {
                    let mut typed = vec![];
                    for (n, t) in scope.iter() {
                        match t {
                            Some(t) => typed.push((n.clone(), t.clone())),
                            None => self.errors.push(syn::Error::new(n.span(), format!(
                                "`{n}` lives across a stop: give it a type (`let {n}: T = …`) — \
                                it is a field of the rest"))),
                        }
                    }
                    let key = self.stops.len();
                    let b = bind.clone();
                    self.stops.push(Stop {
                        key,
                        call,
                        bind,
                        scope: typed,
                        frames: frames.clone(),
                    });
                    if let Some((n, t)) = b {
                        scope.push((n, Some(t)));
                    }
                }
                Some(Err(e)) => self.errors.push(e),
                None => {
                    if let Stmt::Local(l) = s {
                        if has_stop_stmt(s) {
                            self.errors.push(syn::Error::new_spanned(
                                s,
                                "a stop inside an expression: write `let x: T = ask!(…);` first",
                            ));
                        }
                        let (n, t) = match &l.pat {
                            Pat::Type(pt) => match &*pt.pat {
                                Pat::Ident(pi) => (Some(pi.ident.clone()), Some((*pt.ty).clone())),
                                _ => (None, None),
                            },
                            Pat::Ident(pi) => (Some(pi.ident.clone()), None),
                            _ => (None, None),
                        };
                        if let Some(n) = n {
                            scope.push((n, t));
                        }
                    } else if has_stop_stmt(s) {
                        match s {
                            Stmt::Expr(e, _) => self.find_expr(e, scope, frames),
                            _ => self.errors.push(syn::Error::new_spanned(
                                s,
                                "a stop inside this statement: not lowered",
                            )),
                        }
                    }
                }
            }
            frames.pop();
        }
        scope.truncate(depth);
    }

    fn find_expr(&mut self, e: &'a Expr, scope: &mut Vec<Var>, frames: &mut Vec<Frame<'a>>) {
        match e {
            Expr::If(i) => {
                if has_stop_expr(&i.cond) {
                    let m = "a stop in a condition: ask first, `let c: bool = ask!(…);`";
                    self.errors.push(syn::Error::new_spanned(&i.cond, m));
                }
                self.find(&i.then_branch.stmts, scope, frames, Owner::Plain);
                if let Some((_, eb)) = &i.else_branch {
                    self.find_expr(eb, scope, frames);
                }
            }
            Expr::Block(b) => self.find(&b.block.stmts, scope, frames, Owner::Plain),
            Expr::While(w) => {
                if w.label.is_some() {
                    self.errors.push(syn::Error::new_spanned(w, "a labelled loop holding a stop: not lowered"));
                }
                if has_stop_expr(&w.cond) {
                    self.errors.push(syn::Error::new_spanned(&w.cond, "a stop in a loop's condition: ask in its body"));
                }
                self.loops += 1;
                let id = self.loops;
                self.find(&w.body.stmts, scope, frames, Owner::Loop(e, id));
            }
            Expr::Loop(l) => {
                if l.label.is_some() {
                    self.errors.push(syn::Error::new_spanned(l, "a labelled loop holding a stop: not lowered"));
                }
                self.loops += 1;
                let id = self.loops;
                self.find(&l.body.stmts, scope, frames, Owner::Loop(e, id));
            }
            Expr::Match(m) => {
                if has_stop_expr(&m.expr) {
                    self.errors.push(syn::Error::new_spanned(&m.expr, "a stop in a match's scrutinee: ask first"));
                }
                for arm in &m.arms {
                    if arm.guard.as_ref().is_some_and(|(_, g)| has_stop_expr(g)) {
                        self.errors.push(syn::Error::new_spanned(arm, "a stop in a guard: not lowered"));
                    }
                    match &*arm.body {
                        Expr::Block(b) => self.find(&b.block.stmts, scope, frames, Owner::Plain),
                        other if has_stop_expr(other) => {
                            let m = "a stop in a match arm: write the arm as a block `{ … }`";
                            self.errors.push(syn::Error::new_spanned(other, m))
                        }
                        _ => {}
                    }
                }
            }
            Expr::ForLoop(f) => {
                let m = "a `for` holding a stop: write it as a `while` (its iterator cannot be kept in the rest)";
                self.errors.push(syn::Error::new_spanned(f.for_token, m))
            }
            other => {
                let m = "a stop inside this expression: not lowered \
                         (statements, if, while, loop, match arms and blocks are)";
                self.errors.push(syn::Error::new_spanned(other, m))
            }
        }
    }

    fn stop_key(&self, s: &Stmt) -> Option<usize> {
        // a stop statement is found again by where it is: the same statement in the same block
        let p = s as *const Stmt;
        self.stops
            .iter()
            .find(|st| {
                let f = st.frames.last().unwrap();
                std::ptr::eq(&f.stmts[f.index], p)
            })
            .map(|st| st.key)
    }

    // THE CODE: statements as they are, a stop the step returned with its rest; `target`: the labels `break` and
    // `continue` of the loop being resumed go to (none in a loop written out again: there they are its own)
    fn gen_stmts(&self, stmts: &[Stmt], target: Option<usize>) -> TokenStream2 {
        let mut out = TokenStream2::new();
        for s in stmts {
            out.extend(self.gen_stmt(s, target));
            // a stop returned: the statements after it are its rest's
            if self.stop_key(s).is_some() {
                break;
            }
        }
        out
    }

    fn gen_stmt(&self, s: &Stmt, target: Option<usize>) -> TokenStream2 {
        if let Some(k) = self.stop_key(s) {
            let st = &self.stops[k];
            let state = format_ident!("S{}", k + 1);
            let name = &self.name;
            let names = st.scope.iter().map(|(n, _)| n);
            let func = &st.call.func;
            let args = st.call.args.iter();
            let ops = &self.ops;
            return quote!(
                return ::okay::resumable::Step::Perform(
                    <#ops as ::core::convert::From<_>>::from(#func(#(#args,)* ::okay::Reply::new())),
                    #name::#state { #(#names),* },
                );
            );
        }
        if let (Stmt::Expr(e, semi), true) = (s, has_stop_stmt(s)) {
            let code = self.gen_expr(e, target);
            return quote!(#code #semi);
        }
        // no stop: copied, `return` the program's Done, the resumed loop's break and continue its labels
        let mut s = s.clone();
        Rewrite { target, depth: 0 }.visit_stmt_mut(&mut s);
        quote!(#s)
    }

    fn gen_block(&self, b: &Block, target: Option<usize>) -> TokenStream2 {
        let code = self.gen_stmts(&b.stmts, target);
        quote!({ #code })
    }

    fn gen_expr(&self, e: &Expr, target: Option<usize>) -> TokenStream2 {
        match e {
            Expr::If(i) => {
                let cond = &i.cond;
                let then_b = self.gen_block(&i.then_branch, target);
                let els = i.else_branch.as_ref().map(|(_, eb)| {
                    let c = self.gen_expr(eb, target);
                    quote!(else #c)
                });
                quote!(if #cond #then_b #els)
            }
            Expr::Block(b) => self.gen_block(&b.block, target),
            // a loop written out again: its break and continue its own
            Expr::While(w) => {
                let cond = &w.cond;
                let body = self.gen_block(&w.body, None);
                quote!(while #cond #body)
            }
            Expr::Loop(l) => {
                let body = self.gen_block(&l.body, None);
                quote!(loop #body)
            }
            Expr::Match(m) => {
                let scrut = &m.expr;
                let arms = m.arms.iter().map(|arm| {
                    let pat = &arm.pat;
                    let guard = arm.guard.as_ref().map(|(_, g)| quote!(if #g));
                    let body = match &*arm.body {
                        Expr::Block(b) => self.gen_block(&b.block, target),
                        other => {
                            let mut o = other.clone();
                            Rewrite { target, depth: 0 }.visit_expr_mut(&mut o);
                            quote!(#o)
                        }
                    };
                    quote!(#pat #guard => #body,)
                });
                quote!(match #scrut { #(#arms)* })
            }
            other => quote!(#other),
        }
    }

    /// the rest of stop `k`: its answer bound, then from the stop on — the rest of each block around it, innermost
    /// first; a loop's body finished, the loop entered again — to the program's end
    fn gen_rest(&self, k: usize) -> TokenStream2 {
        let st = &self.stops[k];
        let answer = quote!(okay_answer);
        let mut code = match &st.bind {
            Some((n, t)) => quote!(let mut #n: #t = ::okay::Reply::<#t>::new().take(#answer);),
            None => quote!(let _ = #answer;),
        };
        let frames = &st.frames;
        for j in (0..frames.len()).rev() {
            let f = &frames[j];
            // the loop being resumed nearest around this block: its labels
            let target = frames[..=j].iter().rev().find_map(|f| match f.owner {
                Owner::Loop(_, id) => Some(id),
                _ => None,
            });
            let rest = self.gen_stmts(&f.stmts[f.index + 1..], target);
            code = quote!(#code #rest);
            match f.owner {
                Owner::Loop(l, id) => {
                    let (b, c) = (format_ident!("okay_b{}", id), format_ident!("okay_c{}", id));
                    let again = self.gen_expr(l, None);
                    let b = syn::Lifetime::new(&format!("'{b}"), Span::call_site());
                    let c = syn::Lifetime::new(&format!("'{c}"), Span::call_site());
                    code = quote!(#b: { #c: { #code } #again });
                }
                Owner::Plain => code = quote!({ #code }),
            }
        }
        code
    }
}

/// a statement copied: `return e` the program's Done; with a target, the `break` and `continue` of the loop being
/// resumed (not of a loop inside the statement) its labels
struct Rewrite {
    target: Option<usize>,
    depth: usize,
}
impl VisitMut for Rewrite {
    fn visit_expr_mut(&mut self, e: &mut Expr) {
        match e {
            Expr::Closure(_) => return,
            Expr::Return(r) => {
                if let Some(v) = r.expr.as_mut() {
                    self.visit_expr_mut(v);
                }
                let v = r.expr.take().map(|v| quote!(#v)).unwrap_or(quote!(()));
                *e = syn::parse_quote!(return ::okay::resumable::Step::Done(#v));
                return;
            }
            Expr::While(_) | Expr::Loop(_) | Expr::ForLoop(_) => {
                self.depth += 1;
                syn::visit_mut::visit_expr_mut(self, e);
                self.depth -= 1;
                return;
            }
            Expr::Break(b) if self.depth == 0 && b.label.is_none() && b.expr.is_none() => {
                if let Some(id) = self.target {
                    let l = syn::Lifetime::new(&format!("'okay_b{id}"), Span::call_site());
                    *e = syn::parse_quote!(break #l);
                }
                return;
            }
            Expr::Continue(c) if self.depth == 0 && c.label.is_none() => {
                if let Some(id) = self.target {
                    let l = syn::Lifetime::new(&format!("'okay_c{id}"), Span::call_site());
                    *e = syn::parse_quote!(break #l);
                }
                return;
            }
            _ => {}
        }
        syn::visit_mut::visit_expr_mut(self, e);
    }
    fn visit_item_mut(&mut self, _: &mut syn::Item) {}
}

fn generate(ops: Type, f: ItemFn) -> syn::Result<TokenStream2> {
    if !f.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &f.sig.generics,
            "a resumable fn has no type parameters (yet)",
        ));
    }
    let vis = &f.vis;
    let fname = &f.sig.ident;
    let mut cname = String::new();
    for part in fname.to_string().split('_') {
        let mut cs = part.chars();
        if let Some(c) = cs.next() {
            cname.extend(c.to_uppercase());
            cname.extend(cs);
        }
    }
    let name = Ident::new(&format!("{cname}Rest"), Span::call_site());
    let out: Type = match &f.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, t) => (**t).clone(),
    };
    let mut params: Vec<Var> = vec![];
    for a in &f.sig.inputs {
        let FnArg::Typed(pt) = a else {
            return Err(syn::Error::new_spanned(a, "a resumable fn takes no self"));
        };
        let Pat::Ident(pi) = &*pt.pat else {
            return Err(syn::Error::new_spanned(&pt.pat, "a parameter is a name"));
        };
        params.push((pi.ident.clone(), Some((*pt.ty).clone())));
    }
    // the body: its tail expression the program's value
    let mut stmts = f.block.stmts.clone();
    if let Some(Stmt::Expr(e, None)) = stmts
        .last()
        .filter(|s| !matches!(s, Stmt::Expr(e, None) if has_stop_expr(e)))
    {
        let e = e.clone();
        stmts.pop();
        stmts.push(syn::parse_quote!(return #e;));
    }
    let mut g = Gen {
        name: name.clone(),
        ops: ops.clone(),
        stops: vec![],
        loops: 0,
        errors: vec![],
    };
    let mut scope = params.clone();
    let mut frames = vec![];
    g.find(&stmts, &mut scope, &mut frames, Owner::Plain);
    if let Some(mut e) = g.errors.pop() {
        for more in g.errors.drain(..) {
            e.combine(more);
        }
        return Err(e);
    }
    let start = g.gen_stmts(&stmts, None);
    let end = quote!(
        #[allow(unreachable_code)]
        return ::okay::resumable::Step::Done(::core::default::Default::default());
    );
    let variants = g.stops.iter().map(|st| {
        let s = format_ident!("S{}", st.key + 1);
        let fields = st.scope.iter().map(|(n, t)| quote!(#n: #t));
        quote!(#s { #(#fields),* })
    });
    let arms = g.stops.iter().map(|st| {
        let s = format_ident!("S{}", st.key + 1);
        let names = st.scope.iter().map(|(n, _)| n);
        let rest = g.gen_rest(st.key);
        quote!(#name::#s { #(mut #names),* } => { #rest #end })
    });
    let pdecls = params.iter().map(|(n, t)| quote!(#n: #t));
    let attrs = &f.attrs;
    let empty = g.stops.is_empty();
    let resume_body = if empty {
        quote!(match self {})
    } else {
        quote!(match self { #(#arms)* })
    };
    Ok(quote! {
        /// the rest of a resumable fn: which stop it is at, and the variables in scope there — resumed by
        /// `Rest::resume`, cloned to resume again
        #[derive(Clone)]
        #[allow(non_camel_case_types, dead_code)]
        #vis enum #name { #(#variants),* }

        impl ::okay::resumable::Rest for #name {
            type Op = #ops;
            type Out = #out;
            #[allow(unused_mut, unused_variables, unreachable_code, unused_labels, clippy::needless_return)]
            fn resume(self, okay_answer: ::okay::Answer) -> ::okay::resumable::Step<#ops, #name, #out> {
                #resume_body
            }
        }

        #(#attrs)*
        #[allow(unused_mut, unreachable_code, clippy::needless_return)]
        #vis fn #fname(#(#pdecls),*) -> ::okay::resumable::Step<#ops, #name, #out> {
            #start
            #end
        }
    })
}
