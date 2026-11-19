//! Addressing steps inside the nested step tree.

use autoflow_core::Step;

/// Path to a step list: each element is (step index, child list index) from the root.
pub type ListPath = Vec<(usize, usize)>;

/// Path to a single step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepPath {
    pub parents: ListPath,
    pub index: usize,
}

/// What is selected in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    None,
    Step(StepPath),
    /// A child list header such as `then` / `else` / `steps` (palette inserts into it).
    List(ListPath),
}

pub fn list_mut<'a>(root: &'a mut Vec<Step>, parents: &[(usize, usize)]) -> Option<&'a mut Vec<Step>> {
    let mut cur = root;
    for &(i, l) in parents {
        let step = cur.get_mut(i)?;
        cur = step.action.children_mut().into_iter().nth(l)?;
    }
    Some(cur)
}

pub fn list_ref<'a>(root: &'a Vec<Step>, parents: &[(usize, usize)]) -> Option<&'a Vec<Step>> {
    let mut cur = root;
    for &(i, l) in parents {
        let step = cur.get(i)?;
        cur = step.action.children().into_iter().nth(l)?;
    }
    Some(cur)
}

pub fn step_mut<'a>(root: &'a mut Vec<Step>, p: &StepPath) -> Option<&'a mut Step> {
    list_mut(root, &p.parents)?.get_mut(p.index)
}

pub fn step_ref<'a>(root: &'a Vec<Step>, p: &StepPath) -> Option<&'a Step> {
    list_ref(root, &p.parents)?.get(p.index)
}

/// Insert a step relative to the selection; returns the new step's path.
pub fn insert(root: &mut Vec<Step>, sel: &Selection, step: Step) -> StepPath {
    let (parents, index) = match sel {
        Selection::Step(p) => (p.parents.clone(), p.index + 1),
        Selection::List(lp) => {
            let len = list_ref(root, lp).map(|l| l.len()).unwrap_or(0);
            (lp.clone(), len)
        }
        Selection::None => (vec![], root.len()),
    };
    match list_mut(root, &parents) {
        Some(list) => {
            let index = index.min(list.len());
            list.insert(index, step);
            StepPath { parents, index }
        }
        None => {
            root.push(step);
            StepPath { parents: vec![], index: root.len() - 1 }
        }
    }
}
