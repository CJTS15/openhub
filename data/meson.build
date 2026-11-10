desktop_file = configure_file(
  input: 'openhub.desktop.in',
  output: app_id + '.desktop',
  configuration: {'APP_ID': app_id},
  install: true,
  install_dir: datadir / 'applications',
)

# Window size and the last page; compiled by gnome.post_install.
configure_file(
  input: 'openhub.gschema.xml.in',
  output: app_id + '.gschema.xml',
  configuration: {'APP_ID': app_id},
  install: true,
  install_dir: datadir / 'glib-2.0/schemas',
)

metainfo_file = configure_file(
  input: 'openhub.metainfo.xml.in',
  output: app_id + '.metainfo.xml',
  configuration: {'APP_ID': app_id},
  install: true,
  install_dir: datadir / 'metainfo',
)

install_data(
  'icons/hicolor/scalable/apps/openhub.svg',
  rename: app_id + '.svg',
  install_dir: datadir / 'icons/hicolor/scalable/apps',
)
install_data(
  'icons/hicolor/symbolic/apps/openhub-symbolic.svg',
  rename: app_id + '-symbolic.svg',
  install_dir: datadir / 'icons/hicolor/symbolic/apps',
)

# Logitech and Razer mice and VIA keyboards: user access to hidraw, and
# restoring Logitech settings when they connect. A Flatpak can't install these; its users copy them by hand.
install_data(
  'udev/70-openhub-logitech.rules',
  install_dir: get_option('prefix') / 'lib/udev/rules.d',
)
install_data(
  'udev/70-openhub-razer.rules',
  'udev/70-openhub-via.rules',
  'udev/70-openhub-ddc.rules',
  install_dir: get_option('prefix') / 'lib/udev/rules.d',
)
# DDC/CI to external monitors goes through /dev/i2c-*, which needs i2c-dev.
install_data(
  'modules-load.d/openhub-i2c.conf',
  install_dir: get_option('prefix') / 'lib/modules-load.d',
)
install_data(
  'systemd/openhub-mouse@.service',
  install_dir: get_option('prefix') / 'lib/systemd/user',
)

desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('validate-desktop', desktop_file_validate, args: [desktop_file])
endif

appstreamcli = find_program('appstreamcli', required: false)
if appstreamcli.found()
  test(
    'validate-metainfo',
    appstreamcli,
    args: ['validate', '--no-net', '--explain', metainfo_file],
  )
endif

glib_compile_schemas = find_program('glib-compile-schemas', required: false)
if glib_compile_schemas.found()
  test(
    'validate-schema',
    glib_compile_schemas,
    args: ['--strict', '--dry-run', meson.current_build_dir()],
  )
endif
