use super::*;

/// One bit per input cell of `builder`.
fn input_bits(builder: &Builder, count: u32) -> Vec<Bit> {
    (0..count)
        .map(|index| Bit::assume(builder.input(index).unwrap()))
        .collect()
}

/// `count` cells holding the bits of `assignment`, low bit first.
fn cells(assignment: u32, count: u32) -> Vec<f32> {
    (0..count)
        .map(|index| f32::from(assignment >> index & 1 == 1))
        .collect()
}

/// Builds over `inputs` bits, then evaluates the produced bit for every
/// assignment of those inputs, so a construction is checked against its whole
/// truth table rather than one case.
fn truth_table(inputs: u32, build: impl FnOnce(&mut Builder, &[Bit]) -> Bit) -> Vec<bool> {
    let mut builder = Builder::new(inputs);
    let bits: Vec<Bit> = input_bits(&builder, inputs);
    let result = build(&mut builder, &bits);
    let circuit = builder.finish();

    (0..1 << inputs)
        .map(|assignment| {
            let cells: Vec<f32> = cells(assignment, inputs);
            let values = circuit.evaluate(&cells).expect("one cell per input");
            result.form().value(&values).unwrap() != 0.0
        })
        .collect()
}

#[test]
fn not_is_free_and_correct() {
    let builder = Builder::new(1);
    let bit = Bit::assume(builder.input(0).unwrap());
    let before = builder.circuit().ops().len();
    let _ = bit.not();
    assert_eq!(
        builder.circuit().ops().len(),
        before,
        "not appends no operation"
    );

    assert_eq!(
        truth_table(1, |_, bits| bits[0].clone().not()),
        [true, false]
    );
}

#[test]
fn and_or_follow_their_truth_tables() {
    // Assignment order is (a, b) = 00, 01, 10, 11 with a the low bit.
    assert_eq!(
        truth_table(2, |builder, bits| builder.and(&bits[0], &bits[1]).unwrap()),
        [false, false, false, true],
    );
    assert_eq!(
        truth_table(2, |builder, bits| builder.or(&bits[0], &bits[1]).unwrap()),
        [false, true, true, true],
    );
}

#[test]
fn or_disjoint_is_free() {
    let builder = Builder::new(2);
    let bits: Vec<Bit> = input_bits(&builder, 2);
    let before = builder.circuit().ops().len();
    let _ = Bit::or_disjoint(bits);
    assert_eq!(
        builder.circuit().ops().len(),
        before,
        "a disjoint or is addition"
    );
}

#[test]
fn is_zero_costs_one_operation_and_detects_zero() {
    let mut builder = Builder::new(3);
    let bits: Vec<Bit> = input_bits(&builder, 3);
    let count = Count::of(bits);
    let before = builder.circuit().ops().len();
    let zero = builder.is_zero(&count).unwrap();
    assert_eq!(
        builder.circuit().ops().len() - before,
        1,
        "testing a count against zero is a single rectified gate",
    );

    let circuit = builder.finish();
    for assignment in 0..8 {
        let cells: Vec<f32> = cells(assignment, 3);
        let values = circuit.evaluate(&cells).expect("three cells");
        let expected = assignment == 0;
        assert_eq!(
            zero.form().value(&values).unwrap() != 0.0,
            expected,
            "assignment {assignment}"
        );
    }
}

#[test]
fn at_least_counts_set_bits() {
    for threshold in 0..=3 {
        #[expect(clippy::cast_precision_loss, reason = "a tiny threshold")]
        let wanted = threshold as f32;
        let table = truth_table(3, |builder, bits| {
            let count = Count::of(bits.to_vec());
            builder.at_least(&count, wanted).unwrap()
        });
        for (assignment, got) in table.iter().enumerate() {
            let set = u32::try_from(assignment).expect("small").count_ones();
            assert_eq!(
                *got,
                set >= threshold,
                "at_least {threshold}, assignment {assignment}"
            );
        }
    }
}

#[test]
fn shortfall_is_the_complement_of_a_count() {
    let builder = Builder::new(2);
    let bits: Vec<Bit> = input_bits(&builder, 2);
    let shortfall = Count::shortfall(2.0, bits);
    let circuit = builder.finish();
    let values = circuit.evaluate(&[1.0, 0.0]).expect("two cells");
    assert_eq!(shortfall.form().value(&values).unwrap(), 1.0);
}

#[test]
fn none_earlier_marks_positions_with_nothing_set_before_them() {
    let mut builder = Builder::new(3);
    let bits: Vec<Bit> = input_bits(&builder, 3);
    let before = builder.circuit().ops().len();
    let none_earlier = builder.none_earlier(&bits).unwrap();
    assert_eq!(
        builder.circuit().ops().len() - before,
        2,
        "one gate per position, and the first has nothing to test",
    );

    let circuit = builder.finish();
    for assignment in 0..8u32 {
        let values = circuit
            .evaluate(&cells(assignment, 3))
            .expect("three cells");
        for (index, bit) in none_earlier.iter().enumerate() {
            let expected = (0..index).all(|earlier| assignment >> earlier & 1 == 0);
            assert_eq!(
                bit.form().value(&values).unwrap() != 0.0,
                expected,
                "assignment {assignment}, position {index}",
            );
        }
    }
}

#[test]
fn none_earlier_with_the_bits_picks_exactly_the_first_one() {
    // This is how an ordered composite finds the child that stopped the walk,
    // so the one-hot property is the lowering's correctness, not a nicety.
    let mut builder = Builder::new(3);
    let bits: Vec<Bit> = input_bits(&builder, 3);
    let none_earlier = builder.none_earlier(&bits).unwrap();
    let mut picked = Vec::new();
    for (bit, none) in bits.iter().zip(&none_earlier) {
        picked.push(builder.and(bit, none).unwrap());
    }
    let circuit = builder.finish();

    for assignment in 0..8u32 {
        let values = circuit
            .evaluate(&cells(assignment, 3))
            .expect("three cells");
        let set: Vec<bool> = picked
            .iter()
            .map(|bit| bit.form().value(&values).unwrap() != 0.0)
            .collect();
        assert_eq!(
            set.iter().position(|chosen| *chosen),
            (0..3).find(|index| assignment >> index & 1 == 1),
            "assignment {assignment} picked {set:?}",
        );
        assert!(
            set.iter().filter(|chosen| **chosen).count() <= 1,
            "the selection must be one-hot"
        );
    }
}

#[test]
fn constants_fold_without_appending_operations() {
    let mut builder = Builder::new(1);
    let before = builder.circuit().ops().len();

    assert_eq!(
        builder
            .is_zero(&Count::constant(0.0))
            .unwrap()
            .as_constant(),
        Some(true)
    );
    assert_eq!(
        builder
            .is_zero(&Count::constant(2.0))
            .unwrap()
            .as_constant(),
        Some(false)
    );
    assert_eq!(
        builder
            .at_least(&Count::constant(2.0), 2.0)
            .unwrap()
            .as_constant(),
        Some(true)
    );
    assert_eq!(
        builder.and(&Bit::ONE, &Bit::ONE).unwrap().as_constant(),
        Some(true)
    );
    assert_eq!(
        builder.and(&Bit::ZERO, &Bit::ONE).unwrap().as_constant(),
        Some(false)
    );
    assert_eq!(
        builder.or(&Bit::ZERO, &Bit::ZERO).unwrap().as_constant(),
        Some(false)
    );

    assert_eq!(
        builder.circuit().ops().len(),
        before,
        "folding must not emit anything"
    );
}

/// The construction `none_earlier` replaced: every prefix sum read inline.
fn none_earlier_inline(builder: &mut Builder, bits: &[Bit]) -> Vec<Bit> {
    let mut prefix = Count::constant(0.0);
    let mut result = Vec::new();
    for bit in bits {
        result.push(builder.is_zero(&prefix).unwrap());
        prefix = Count(prefix.0.plus(&bit.0));
    }
    result
}

/// A mix of wire, negated, constant, and composite bit forms over `width` inputs.
fn mixed_bits(builder: &mut Builder, width: u32) -> Vec<Bit> {
    let wires = input_bits(builder, width);
    (0..wires.len())
        .map(|index| match index % 5 {
            0 => wires[index].clone(),
            1 => !wires[index].clone(),
            2 => Bit::ZERO,
            3 => builder.or(&wires[index], &wires[index - 1]).unwrap(),
            _ => !builder.and(&wires[index], &wires[index - 2]).unwrap(),
        })
        .collect()
}

/// Every stored affine term in `circuit`.
fn stored_terms(circuit: &Circuit) -> usize {
    circuit
        .ops()
        .iter()
        .map(|op| match op {
            Op::Affine(form) => form.terms().len(),
            Op::Reglu { left, gate } => left.terms().len() + gate.terms().len(),
        })
        .sum()
}

#[test]
fn none_earlier_in_blocks_agrees_with_the_inline_sums_on_every_assignment() {
    for radix in [2, 3, 4] {
        for width in 0..=9u32 {
            let mut blocked = Builder::new(width);
            let bits = mixed_bits(&mut blocked, width);
            let got = blocked.none_earlier_in_blocks(&bits, radix).unwrap();
            let blocked = blocked.finish();

            let mut inline = Builder::new(width);
            let bits = mixed_bits(&mut inline, width);
            let want = none_earlier_inline(&mut inline, &bits);
            let inline = inline.finish();

            for assignment in 0..1u32 << width {
                let cells = cells(assignment, width);
                let got_values = blocked.evaluate(&cells).unwrap();
                let want_values = inline.evaluate(&cells).unwrap();
                for (index, (got, want)) in got.iter().zip(&want).enumerate() {
                    assert_eq!(
                        got.form().value(&got_values),
                        want.form().value(&want_values),
                        "radix {radix}, width {width}, assignment {assignment}, position {index}",
                    );
                }
            }
        }
    }
}

#[test]
fn none_earlier_up_to_the_radix_builds_the_inline_circuit() {
    for width in [1, 2, 17, u32::try_from(PREFIX_RADIX).unwrap()] {
        let mut blocked = Builder::new(width);
        let bits = mixed_bits(&mut blocked, width);
        let got = blocked.none_earlier(&bits).unwrap();

        let mut inline = Builder::new(width);
        let bits = mixed_bits(&mut inline, width);
        let want = none_earlier_inline(&mut inline, &bits);

        assert_eq!(got, want, "width {width}");
        assert_eq!(blocked.finish(), inline.finish(), "width {width}");
    }
}

#[test]
fn none_earlier_stores_terms_linear_in_width() {
    for width in [256u32, 512, 1024, 2048, 4096] {
        let mut builder = Builder::new(width);
        let bits = input_bits(&builder, width);
        builder.none_earlier(&bits).unwrap();
        let terms = stored_terms(builder.circuit());
        let per_bit = 2 * PREFIX_RADIX;
        assert!(
            terms <= width as usize * per_bit,
            "width {width} stores {terms} terms, over {per_bit} per bit",
        );
    }
}

#[test]
fn none_earlier_past_the_radix_costs_one_layer_per_level() {
    for (width, depth) in [(64u32, 1), (65, 2), (4096, 2), (4097, 3)] {
        let mut builder = Builder::new(width);
        let bits = input_bits(&builder, width);
        builder.none_earlier(&bits).unwrap();
        assert_eq!(builder.circuit().depth(), depth, "width {width}");
    }
}
