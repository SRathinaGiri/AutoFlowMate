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
    /// A connector in the flowchart: insert at this index of this list.
    Gap(ListPath, usize),
}

/// Something being dragged in the editor.
#[derive(Debug, Clone)]
pub enum DragItem {
    /// A new action from the palette.
    New(String),
    /// An existing step.
    Move(StepPath),
}

/// Move a step to `index` of list `to`. Returns the new path, or None when the
/// move is impossible (e.g. into its own children).
pub fn move_step(root: &mut Vec<Step>, from: &StepPath, to: &ListPath, index: usize) -> Option<StepPath> {
    // Refuse to move a container into itself.
    let n = from.parents.len();
    if to.len() > n && to[..n] == from.parents[..] && to[n].0 == from.index {
        return None;
    }
    let step = list_mut(root, &from.parents)?.remove(from.index);
    let mut to = to.clone();
    let mut index = index;
    if to == from.parents && index > from.index {
        index -= 1;
    } else if to.len() > n && to[..n] == from.parents[..] && to[n].0 > from.index {
        to[n].0 -= 1;
    }
    let list = list_mut(root, &to)?;
    let index = index.min(list.len());
    list.insert(index, step);
    Some(StepPath { parents: to, index })
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
        Selection::Gap(lp, i) => (lp.clone(), *i),
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

#[cfg(test)]
mod tests {
    use super::*;
    use autoflow_core::Step;

    fn flow() -> Vec<Step> {
        serde_json::from_value(serde_json::json!([
            {"id":"a","action":"log"},
            {"id":"loop","action":"repeat","steps":[{"id":"in1","action":"log"}]},
            {"id":"c","action":"log"}
        ]))
        .unwrap()
    }
    fn ids(v: &[Step]) -> Vec<String> {
        v.iter().map(|s| s.id.clone()).collect()
    }

    #[test]
    fn move_within_list() {
        let mut f = flow();
        let p = move_step(&mut f, &StepPath { parents: vec![], index: 0 }, &vec![], 3).unwrap();
        assert_eq!(ids(&f), ["loop", "c", "a"]);
        assert_eq!(p.index, 2);
    }

    #[test]
    fn move_into_and_out_of_loop() {
        let mut f = flow();
        // "c" into the loop body at index 0
        move_step(&mut f, &StepPath { parents: vec![], index: 2 }, &vec![(1, 0)], 0).unwrap();
        assert_eq!(ids(f[1].action.children()[0]), ["c", "in1"]);
        // "a" (before the loop) into the loop: the loop shifts from index 1 to 0
        move_step(&mut f, &StepPath { parents: vec![], index: 0 }, &vec![(1, 0)], 2).unwrap();
        assert_eq!(ids(&f), ["loop"]);
        assert_eq!(ids(f[0].action.children()[0]), ["c", "in1", "a"]);
    }

    #[test]
    fn cannot_move_into_itself() {
        let mut f = flow();
        assert!(move_step(&mut f, &StepPath { parents: vec![], index: 1 }, &vec![(1, 0)], 0).is_none());
        assert_eq!(ids(&f), ["a", "loop", "c"]);
    }
}
