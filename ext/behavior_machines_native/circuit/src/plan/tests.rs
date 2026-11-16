use super::*;

use alloc::collections::BTreeSet;
use alloc::vec;

/// Three nodes, one leaf at node 1, one cursor owned by node 0.
fn plan() -> Plan {
    Plan::new(3, vec![1], vec![0], vec![1, 2, 0])
}

#[test]
fn counts_report_the_shape() {
    let plan = plan();
    assert_eq!(plan.node_count(), 3);
    assert_eq!(plan.leaf_count(), 1);
    assert_eq!(plan.cursor_count(), 1);
    // One flag per node, one cursor, three status cells for the one leaf.
    assert_eq!(plan.cell_count(), 3 + 1 + 3);
}

#[test]
fn every_cell_is_addressed_exactly_once() {
    let plan = plan();
    let mut cells = BTreeSet::new();
    for node in 0..plan.node_count() {
        assert!(
            cells.insert(plan.running_cell(node).unwrap()),
            "running cell {node} collides"
        );
    }
    for cursor in 0..plan.cursor_count() {
        assert!(
            cells.insert(plan.cursor_cell(cursor).unwrap()),
            "cursor cell {cursor} collides"
        );
    }
    for slot in 0..plan.leaf_count() {
        for status in [Status::Success, Status::Failure, Status::Running] {
            let cell = plan.leaf_status_cell(slot, status).unwrap();
            assert!(cells.insert(cell), "leaf cell {slot}/{status:?} collides");
        }
    }
    assert_eq!(
        cells.len(),
        plan.cell_count() as usize,
        "the layout must cover its own cell count with no gaps",
    );
    assert!(
        cells.iter().all(|cell| *cell < plan.cell_count()),
        "every cell must be inside the input width",
    );
}

#[test]
fn a_leaf_slot_maps_back_to_its_node() {
    let plan = plan();
    assert_eq!(plan.leaf_node(0), Some(1));
    assert_eq!(plan.leaf_slot(1), Some(0));
    assert_eq!(plan.leaf_slot(0), None, "node 0 is not a leaf");
    assert_eq!(plan.leaf_node(9), None);
}

#[test]
fn a_cursor_cell_maps_back_to_its_node() {
    let plan = plan();
    assert_eq!(plan.cursor_node(0), Some(0));
    assert_eq!(plan.cursor_node(1), None);
}

#[test]
fn the_three_leaf_statuses_are_adjacent_and_ordered() {
    let plan = plan();
    let base = plan.leaf_cell(0).unwrap();
    assert_eq!(plan.leaf_status_cell(0, Status::Success), Some(base));
    assert_eq!(plan.leaf_status_cell(0, Status::Failure), Some(base + 1));
    assert_eq!(plan.leaf_status_cell(0, Status::Running), Some(base + 2));
}

#[test]
fn an_index_outside_the_plan_names_no_cell() {
    let plan = plan();
    assert_eq!(plan.running_cell(3), None);
    assert_eq!(plan.cursor_cell(1), None);
    assert_eq!(plan.leaf_cell(1), None);
    assert_eq!(plan.leaf_status_cell(1, Status::Running), None);
    // Far out of range must answer, not overflow.
    assert_eq!(plan.running_cell(usize::MAX), None);
    assert_eq!(plan.cursor_cell(usize::MAX), None);
    assert_eq!(plan.leaf_status_cell(usize::MAX, Status::Running), None);
}

#[test]
fn the_tick_cells_follow_the_addressed_layout() {
    let plan = plan();
    let cells = plan.cells(&[true, false, true], &[7], &[Status::Failure]);
    assert_eq!(cells.len(), plan.cell_count() as usize);
    let at = |cell: Option<u32>| cells[cell.unwrap() as usize];
    assert_eq!(at(plan.running_cell(0)), 1.0);
    assert_eq!(at(plan.running_cell(1)), 0.0);
    assert_eq!(at(plan.running_cell(2)), 1.0);
    assert_eq!(at(plan.cursor_cell(0)), 7.0);
    assert_eq!(at(plan.leaf_status_cell(0, Status::Success)), 0.0);
    assert_eq!(at(plan.leaf_status_cell(0, Status::Failure)), 1.0);
    assert_eq!(at(plan.leaf_status_cell(0, Status::Running)), 0.0);
}

#[test]
fn a_cursor_past_u16_is_clamped_into_its_cell() {
    let plan = plan();
    let cells = plan.cells(&[false; 3], &[u32::MAX], &[Status::Running]);
    assert_eq!(
        cells[plan.cursor_cell(0).unwrap() as usize],
        f32::from(u16::MAX)
    );
}

#[test]
fn the_report_order_lists_every_node_once() {
    let plan = plan();
    let seen: BTreeSet<usize> = plan.report_order().iter().copied().collect();
    assert_eq!(seen.len(), plan.node_count());
    assert_eq!(
        plan.report_order().last(),
        Some(&0),
        "the root reports last"
    );
}
