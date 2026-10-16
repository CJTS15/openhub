desktop_file = configure_file(
  input: 'openhub.desktop.in',
  output: app_id + '.desktop',
  configuration: {'APP_ID': app_id},
  install: true,
  install_dir: datadir / 'applications',
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

# Logitech mice: user access to hidraw, and restoring their settings when
# they connect. A Flatpak can't install these; its users copy them by hand.
install_data(
  'udev/70-openhub-logitech.rules',
  install_dir: get_option('prefix') / 'lib/udev/rules.d',
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
