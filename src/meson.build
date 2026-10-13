cargo = find_program('cargo')

cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', meson.project_build_root() / 'src',
]

if get_option('buildtype') == 'release'
  cargo_options += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

cargo_env = [
  'CARGO_HOME=' + meson.project_build_root() / 'cargo-home',
  'OPENHUB_APP_ID=' + app_id,
  'OPENHUB_VERSION=' + meson.project_version(),
]

custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: meson.project_name(),
  console: true,
  install: true,
  install_dir: bindir,
  command: [
    'env', cargo_env,
    cargo, 'build', cargo_options,
    '&&',
    'cp', 'src' / rust_target / meson.project_name(), '@OUTPUT@',
  ],
)

test(
  'cargo-test',
  cargo,
  args: ['test', cargo_options],
  env: cargo_env,
  timeout: 600,
)
