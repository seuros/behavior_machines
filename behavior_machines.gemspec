# frozen_string_literal: true

require_relative 'lib/behavior_machines/version'

Gem::Specification.new do |spec|
  spec.name = 'behavior_machines'
  spec.version = BehaviorMachines::VERSION
  spec.authors = ['Abdelkader Boudih']
  spec.email = ['terminale@gmail.com']

  spec.summary = 'Behavior trees with explicit halt semantics, following the behavior-tree pattern'
  spec.description = <<~DESC
    BehaviorMachines runs behavior trees whose nodes follow the behavior-tree pattern: Sequence,
    Fallback, their reactive variants, thresholded Parallel,
    IfThenElse, WhileDoElse, and the usual decorators. Every running leaf the tree abandons is halted
    exactly once, in the same call, so leaves can own real resources. Pure Ruby, with an optional
    native backend built on the behavior-machines Rust crate, and leaves for state_machines objects.
  DESC
  spec.homepage = 'https://github.com/seuros/behavior_machines'
  spec.license = 'BSD-3-Clause'
  spec.required_ruby_version = '>= 4.0.0'
  spec.platform = Gem::Platform::RUBY

  spec.metadata['homepage_uri'] = spec.homepage
  spec.metadata['source_code_uri'] = spec.homepage
  spec.metadata['bug_tracker_uri'] = "#{spec.homepage}/issues"
  spec.metadata['rubygems_mfa_required'] = 'true'
  spec.metadata['cargo_crate_name'] = 'behavior_machines_native'
  spec.metadata['cargo_manifest_path'] = 'ext/behavior_machines_native/ffi/Cargo.toml'

  lib_files = Dir['lib/**/*'].reject { |path| path =~ /\.(bundle|so|dll)\z/ }
  spec.files = lib_files + %w[LICENSE README.md]
  spec.require_paths = ['lib']

  spec.add_dependency 'state_machines', '>= 0.202'

  spec.add_development_dependency 'minitest', '~> 6.0'
  spec.add_development_dependency 'rake', '~> 13.0'
  spec.add_development_dependency 'rake-compiler', '~> 1.3'
  spec.add_development_dependency 'rb_sys', '~> 0.9'
end
