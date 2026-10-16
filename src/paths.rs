//! Every path: a continuation resumed with each answer in turn.
//!
//! A future cannot be cloned, so the program is started again with the answers so
//! far replayed (by their index) — for a program pure apart from its effects.

use crate::data::Data;
use crate::kernel::{Asked, Computation, Step};

/// The program's value for every combination of answers, depth first. `answers`
/// gives every answer an operation may have; `program` starts the program anew.
pub fn every_path<A>(answers: impl Fn(&Asked) -> Vec<Data>, program: impl Fn() -> Computation<A>) -> Result<Vec<A>, String> {
    let mut out = Vec::new();
    let mut stack: Vec<Vec<usize>> = vec![vec![]];
    while let Some(path) = stack.pop() {
        let mut c = program();
        let mut s = c.step();
        let mut depth = 0;
        loop {
            match s {
                Step::Done(a) => {
                    out.push(a);
                    break;
                }
                Step::Failed(why) => return Err(why),
                Step::Cancelled => break,
                Step::Yielded(asked) => {
                    let all = answers(&asked);
                    if depth < path.len() {
                        s = c.resume_data(&all[path[depth]]);
                        depth += 1;
                    } else {
                        // branch here: every answer, the first explored first
                        for i in (0..all.len()).rev() {
                            stack.push([path.clone(), vec![i]].concat());
                        }
                        break;
                    }
                }
            }
        }
    }
    Ok(out)
}
