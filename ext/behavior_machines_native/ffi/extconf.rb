# frozen_string_literal: true

# The native extension is required: there is no pure-Ruby fallback, so a
# missing Cargo or rb_sys fails the install instead of degrading it.
require 'mkmf'
require 'pathname'
require 'rb_sys/mkmf'

create_rust_makefile('behavior_machines_native/behavior_machines_native') do |extension|
  ffi_dir = Pathname(__dir__)
  extension.ext_dir = begin
    ffi_dir.relative_path_from(Pathname(Dir.pwd)).to_s
  rescue ArgumentError
    ffi_dir.expand_path.to_s
  end
  extension.profile = ENV.fetch('RB_SYS_CARGO_PROFILE', :release).to_sym
end

makefile = File.join(Dir.pwd, 'Makefile')
manifest_dir = File.expand_path(__dir__)
patched = File.read(makefile).gsub(/^RB_SYS_CARGO_MANIFEST_DIR \?=.*$/, "RB_SYS_CARGO_MANIFEST_DIR ?= #{manifest_dir}")
File.write(makefile, patched)
