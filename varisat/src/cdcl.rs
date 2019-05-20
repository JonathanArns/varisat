//! Conflict driven clause learning.

use partial_ref::{partial, PartialRef};

use varisat_formula::Lit;
use varisat_internal_proof::ProofStep;

use crate::analyze_conflict::analyze_conflict;
use crate::clause::{assess_learned_clause, bump_clause, db, decay_clause_activities};
use crate::context::{parts::*, Context};
use crate::decision::make_decision;
use crate::incremental::{enqueue_assumption, EnqueueAssumption};
use crate::proof;
use crate::prop::{backtrack, enqueue_assignment, propagate, Conflict, Reason};
use crate::simplify::{prove_units, simplify};
use crate::state::SatState;

/// Find a conflict, learn a clause and backtrack.
pub fn conflict_step<'a>(
    mut ctx: partial!(
        Context<'a>,
        mut AnalyzeConflictP,
        mut AssignmentP,
        mut BinaryClausesP,
        mut ClauseActivityP,
        mut ClauseAllocP,
        mut ClauseDbP,
        mut ImplGraphP,
        mut IncrementalP,
        mut ProofP<'a>,
        mut SolverStateP,
        mut TmpDataP,
        mut TrailP,
        mut VsidsP,
        mut WatchlistsP,
        VariablesP,
    ),
) {
    let conflict = find_conflict(ctx.borrow());

    let conflict = match conflict {
        Ok(()) => {
            if ctx.part(ProofP).models_in_proof() {
                let (tmp, mut ctx) = ctx.split_part_mut(TmpDataP);
                let model = &mut tmp.lits;
                model.clear();
                model.extend(
                    ctx.part(AssignmentP)
                        .assignment()
                        .iter()
                        .enumerate()
                        .flat_map(|(index, assignment)| {
                            assignment.map(|polarity| Lit::from_index(index, polarity))
                        }),
                );
                proof::add_step(ctx.borrow(), true, &ProofStep::Model(&model));
            }
            ctx.part_mut(SolverStateP).sat_state = SatState::Sat;
            return;
        }
        Err(FoundConflict::Assumption) => {
            ctx.part_mut(SolverStateP).sat_state = SatState::UnsatUnderAssumptions;
            return;
        }
        Err(FoundConflict::Conflict(conflict)) => conflict,
    };

    let backtrack_to = analyze_conflict(ctx.borrow(), conflict);

    let (analyze, mut ctx) = ctx.split_part(AnalyzeConflictP);

    for &cref in analyze.involved() {
        bump_clause(ctx.borrow(), cref);
    }

    decay_clause_activities(ctx.borrow());

    backtrack(ctx.borrow(), backtrack_to);

    let clause = analyze.clause();

    proof::add_step(
        ctx.borrow(),
        true,
        &ProofStep::AtClause {
            redundant: clause.len() > 2,
            clause: clause.into(),
            propagation_hashes: analyze.clause_hashes(),
        },
    );

    let reason = match clause.len() {
        0 => {
            ctx.part_mut(SolverStateP).sat_state = SatState::Unsat;
            return;
        }
        1 => Reason::Unit,
        2 => {
            ctx.part_mut(BinaryClausesP)
                .add_binary_clause([clause[0], clause[1]]);
            Reason::Binary([clause[1]])
        }
        _ => {
            let header = assess_learned_clause(ctx.borrow(), clause);
            let cref = db::add_clause(ctx.borrow(), header, clause);
            Reason::Long(cref)
        }
    };

    enqueue_assignment(ctx.borrow(), clause[0], reason);
}

/// Return type of [`find_conflict`].
///
/// Specifies whether a conflict was found during propagation or while enqueuing assumptions.
enum FoundConflict {
    Conflict(Conflict),
    Assumption,
}

impl From<Conflict> for FoundConflict {
    fn from(conflict: Conflict) -> FoundConflict {
        FoundConflict::Conflict(conflict)
    }
}

/// Find a conflict.
///
/// Returns `Err` if a conflict was found and `Ok` if a satisfying assignment was found instead.
fn find_conflict<'a>(
    mut ctx: partial!(
        Context<'a>,
        mut AssignmentP,
        mut BinaryClausesP,
        mut ClauseAllocP,
        mut ClauseDbP,
        mut ImplGraphP,
        mut IncrementalP,
        mut ProofP<'a>,
        mut SolverStateP,
        mut TmpDataP,
        mut TrailP,
        mut VsidsP,
        mut WatchlistsP,
        VariablesP,
    ),
) -> Result<(), FoundConflict> {
    loop {
        let propagation_result = propagate(ctx.borrow());

        let new_unit = prove_units(ctx.borrow());

        propagation_result?;

        if new_unit {
            simplify(ctx.borrow());
        }

        match enqueue_assumption(ctx.borrow()) {
            EnqueueAssumption::Enqueued => continue,
            EnqueueAssumption::Conflict => return Err(FoundConflict::Assumption),
            EnqueueAssumption::Done => (),
        }

        if !make_decision(ctx.borrow()) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use proptest::prelude::*;

    use partial_ref::IntoPartialRefMut;

    use varisat_formula::cnf_formula;
    use varisat_formula::test::{sat_formula, sgen_unsat_formula};

    use crate::load::load_clause;
    use crate::state::SatState;

    #[test]
    fn level_0_unsat() {
        let mut ctx = Context::default();
        let mut ctx = ctx.into_partial_ref_mut();

        let formula = cnf_formula![
            1, 2, 3;
            -1;
            1, -2;
            2, -3;
        ];

        for clause in formula.iter() {
            load_clause(ctx.borrow(), clause);
        }

        while ctx.part(SolverStateP).sat_state == SatState::Unknown {
            conflict_step(ctx.borrow());
        }

        assert_eq!(ctx.part(SolverStateP).sat_state, SatState::Unsat);
    }

    proptest! {
        #[test]
        fn sgen_unsat(formula in sgen_unsat_formula(1..7usize)) {
            let mut ctx = Context::default();
            let mut ctx = ctx.into_partial_ref_mut();

            for clause in formula.iter() {
                load_clause(ctx.borrow(), clause);
            }

            while ctx.part(SolverStateP).sat_state == SatState::Unknown {
                conflict_step(ctx.borrow());
            }

            prop_assert_eq!(ctx.part(SolverStateP).sat_state, SatState::Unsat);
        }

        #[test]
        fn sat(formula in sat_formula(4..20usize, 10..100usize, 0.05..0.2, 0.9..1.0)) {
            let mut ctx = Context::default();
            let mut ctx = ctx.into_partial_ref_mut();

            for clause in formula.iter() {
                load_clause(ctx.borrow(), clause);
            }

            while ctx.part(SolverStateP).sat_state == SatState::Unknown {
                conflict_step(ctx.borrow());
            }

            prop_assert_eq!(ctx.part(SolverStateP).sat_state, SatState::Sat);

            for clause in formula.iter() {
                prop_assert!(clause.iter().any(|&lit| ctx.part(AssignmentP).lit_is_true(
                    lit.map_var(|user_var| ctx
                        .part(VariablesP)
                        .existing_solver_from_user(user_var))
                )));
            }
        }

        #[test]
        fn sgen_unsat_incremetal_clauses(formula in sgen_unsat_formula(1..7usize)) {
            let mut ctx = Context::default();
            let mut ctx = ctx.into_partial_ref_mut();

            let mut last_state = SatState::Sat;

            for clause in formula.iter() {
                load_clause(ctx.borrow(), clause);
                while ctx.part(SolverStateP).sat_state == SatState::Unknown {
                    conflict_step(ctx.borrow());
                }

                if ctx.part(SolverStateP).sat_state != last_state {
                    prop_assert_eq!(ctx.part(SolverStateP).sat_state, SatState::Unsat);
                    prop_assert_eq!(last_state, SatState::Sat);
                    last_state = ctx.part(SolverStateP).sat_state;
                }
            }

            prop_assert_eq!(last_state, SatState::Unsat);
        }
    }
}
