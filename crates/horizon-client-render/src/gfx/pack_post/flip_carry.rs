//! Programs that write no buffer are dropped, but their forced flips still happen (6.1.6).

use horizon_shaders::{Group, PostProgram};

use super::Pass;

/// `programs` built into passes; `skip` names programs a developer leaves out. A program
/// `build` drops carries its forced `flip.*` bits onto the pass before it: a flip swaps the
/// buffer's two textures, so it composes with that pass's flip, and the dropped pass writes
/// nothing in between. `final`'s flips go with it: they would happen after the screen draw.
pub(super) fn build_passes(
    programs: impl IntoIterator<Item = PostProgram>,
    skip: &str,
    mut build: impl FnMut(PostProgram) -> Option<Pass>,
) -> Vec<Pass> {
    let mut passes = Vec::new();
    for p in programs
        .into_iter()
        .filter(|p| !skip.split(',').any(|s| s == p.name))
    {
        let flips = p
            .flips
            .iter()
            .filter(|f| f.1)
            .fold(0u16, |m, f| m | 1 << f.0);
        let carry = p.group != Group::Final;
        match build(p) {
            Some(pass) => passes.push(pass),
            None => {
                if carry && let Some(previous) = passes.last_mut() {
                    previous.flip ^= flips;
                }
            }
        }
    }
    passes
}
