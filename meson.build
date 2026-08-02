project(
    'dx9-rs',
    ['rust'],
    meson_version: '>=1.12',
    version: '0.1.0',
    license: 'MIT',
)

rust = import('rust')
pkgconfig = import('pkgconfig')

cargo_ws = rust.workspace()
pkg = cargo_ws.package('dx9-rs')

dx9_compiler = pkg.library('dx9_compiler', rust_abi: 'c', install: true)

dx9_compiler_dep = declare_dependency(link_with: dx9_compiler)
meson.override_dependency('dx9-rs', dx9_compiler_dep)

pkgconfig.generate(
    dx9_compiler,
    name: 'dx9_compiler',
    description: 'A dx9 shader compiler written in Rust',
    version: meson.project_version(),
)

