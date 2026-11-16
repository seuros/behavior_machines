# frozen_string_literal: true

require_relative 'lib/behavior_machines/version'

Gem::Specification.new do |spec|
  spec.name = 'behavior_machines'
  spec.version = BehaviorMachines::VERSION
  spec.authors = ['Abdelkader Boudih']
  spec.email = ['terminale@gmail.com']

  spec.summary = 'Behavior trees with explicit halt semantics, on the behavior-machines Rust core'
  spec.description = <<~DESC
    BehaviorMachines runs behavior trees whose nodes follow the behavior-tree pattern: Sequence,
    Fallback, their reactive variants, thresholded Parallel, IfThenElse, WhileDoElse, and the usual
    decorators. Every running leaf the tree abandons is halted exactly once, in the same call, so
    leaves can own real resources. Thin Ruby bindings to the allocation-free behavior-machines Rust
    core, with leaves for state_machines objects.
  DESC
  spec.homepage = 'https://github.com/seuros/behavior_machines'
  spec.license = 'BSD-3-Clause'
  spec.required_ruby_version = '>= 4.0.0'

  spec.metadata['homepage_uri'] = spec.homepage
  spec.metadata['source_code_uri'] = spec.homepage
  spec.metadata['bug_tracker_uri'] = "#{spec.homepage}/issues"
  spec.metadata['rubygems_mfa_required'] = 'true'
  spec.metadata['cargo_crate_name'] = 'behavior_machines_native'
  spec.metadata['cargo_manifest_path'] = 'ext/behavior_machines_native/ffi/Cargo.toml'

  spec.files = Dir.glob(%w[
    lib/**/*.rb sig/**/*.rbs ext/**/*.{rb,rs,toml,md} Cargo.toml
    Cargo.lock README.md LICENSE CHANGELOG.md
  ]).select { |path| File.file?(path) }
    .reject { |path| path.end_with?('/tests.rs') || path.include?('/tests/') || path.end_with?('/test_support.rs') }
  spec.require_paths = ['lib']
  spec.extensions = ['ext/behavior_machines_native/extconf.rb']
  spec.add_dependency 'rb_sys', '~> 0.9', '>= 0.9.130'
end
