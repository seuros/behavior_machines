use super::*;

fn wires() -> (Wire, Wire) {
    let circuit = Circuit::new(2);
    (circuit.input(0).unwrap(), circuit.input(1).unwrap())
}

#[test]
fn affine_terms_are_sorted_combined_and_pruned() {
    let (a, b) = wires();
    let form = Affine::new(1.0, [(b, 2.0), (a, 3.0), (b, -2.0), (a, 1.0)]);
    // b's two terms cancel and drop out; a's combine into one.
    assert_eq!(form.terms(), &[(a, 4.0)]);
    assert_eq!(form.bias(), 1.0);
}

#[test]
fn equal_sums_written_differently_compare_equal() {
    let (a, b) = wires();
    let left = Affine::new(0.0, [(a, 1.0), (b, 1.0)]);
    let right = Affine::new(0.0, [(b, 1.0), (a, 1.0)]);
    assert_eq!(
        left, right,
        "term order must not be part of a form's identity"
    );
}

#[test]
#[expect(clippy::suboptimal_flops, reason = "the expected value is spelled out")]
fn scaling_and_offsetting_follow_the_arithmetic() {
    let (a, _) = wires();
    let form = Affine::new(2.0, [(a, 3.0)]).scaled(-2.0).offset(1.0);
    assert_eq!(
        form.value(&[5.0, 0.0]).unwrap(),
        1.0 + -2.0 * (2.0 + 3.0 * 5.0)
    );
}

#[test]
fn a_form_with_no_terms_is_constant() {
    let (a, _) = wires();
    assert_eq!(Affine::constant(7.0).as_constant(), Some(7.0));
    assert_eq!(Affine::wire(a).as_constant(), None);
    // Scaling to zero prunes the term, which makes the form constant.
    assert_eq!(Affine::wire(a).scaled(0.0).as_constant(), Some(0.0));
}

#[test]
fn reglu_rectifies_its_gate() {
    const CASES: &[(f32, f32, f32)] = &[
        (3.0, 2.0, 6.0),
        (3.0, -2.0, 0.0),
        (-3.0, 2.0, -6.0),
        (3.0, 0.0, 0.0),
    ];
    for (left, gate, expected) in CASES {
        let mut circuit = Circuit::new(2);
        let (a, b) = (circuit.input(0).unwrap(), circuit.input(1).unwrap());
        let out = circuit.push(Op::Reglu {
            left: Affine::wire(a),
            gate: Affine::wire(b),
        });
        let values = circuit.evaluate(&[*left, *gate]).expect("two inputs");
        assert_eq!(
            values[out.position()],
            *expected,
            "left {left}, gate {gate}"
        );
    }
}

#[test]
fn layers_place_an_operation_past_everything_it_reads() {
    let mut circuit = Circuit::new(1);
    let a = circuit.input(0).unwrap();
    let first = circuit.push(Op::Affine(Affine::wire(a)));
    // Independent of `first`, so it shares its layer.
    let sibling = circuit.push(Op::Affine(Affine::wire(a)));
    let dependent = circuit.push(Op::Affine(Affine::new(0.0, [(first, 1.0), (sibling, 1.0)])));

    let layers = circuit.layers();
    assert_eq!(layers[a.position()], 0);
    assert_eq!(layers[first.position()], 1);
    assert_eq!(
        layers[sibling.position()],
        1,
        "independent ops share a layer"
    );
    assert_eq!(layers[dependent.position()], 2);
    assert_eq!(circuit.depth(), 2);
}

#[test]
fn an_empty_circuit_has_no_depth() {
    assert_eq!(Circuit::new(3).depth(), 0);
    assert_eq!(Circuit::new(3).value_count(), 3);
}

#[test]
fn evaluating_with_the_wrong_input_count_is_an_error() {
    let circuit = Circuit::new(2);
    assert_eq!(
        circuit.evaluate(&[1.0]),
        Err(Refusal::InputCount {
            expected: 2,
            given: 1
        }),
    );
}
