# frozen_string_literal: true

# Every stateful node shape over leaves a, b, c, and every pair of their
# outcomes; shared by the lifecycle and reference-parity tests.
module Lifecycle
  extend TestSupport

  BM = BehaviorMachines

  OUTCOMES = %i[running success failure].freeze

  def self.shapes
    abc = -> { %w[a b c].map { |name| leaf(name) } }
    [
      BM.sequence(*abc.call), BM.reactive_sequence(*abc.call),
      BM.fallback(*abc.call), BM.reactive_fallback(*abc.call),
      parallel_all(*abc.call), parallel_any(*abc.call), race(*abc.call),
      BM.parallel(abc.call, 2, 2),
      BM.if_then_else(leaf('a'), leaf('b'), leaf('c')), BM.if_then(leaf('a'), leaf('b')),
      BM.while_do_else(leaf('a'), leaf('b'), leaf('c')), BM.while_do(leaf('a'), leaf('b')),
      BM.sequence(*abc.call).inverter, parallel_all(*abc.call).force_success,
      BM.fallback(*abc.call).force_failure, BM.sequence(*abc.call).timeout(ms(2)),
      BM.sequence(*abc.call).repeat(2), BM.fallback(*abc.call).retry_until_successful(2),
      BM.sequence(*abc.call).keep_running_until_failure,
      BM.sequence(BM.sleep(ms(2)), leaf('a')), BM.fallback(BM.always_failure, leaf('a'), BM.always_success)
    ]
  end

  # Scripts a, b, c from the base-3 digits of `code`.
  def self.script(script, code)
    %w[a b c].each do |name|
      script.set(name, OUTCOMES[code % 3])
      code /= 3
    end
  end
end
