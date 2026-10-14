project(
  'openhub',
  'rust',
  version: '0.2.0',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 1.0.0',
)

# Placeholder until the final app ID is decided; change it here only.
app_id = 'io.github.cjts15.Openhub'

dependency('gtk4', version: '>= 4.14')
dependency('libadwaita-1', version: '>= 1.5')

prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')

subdir('data')
subdir('src')

gnome = import('gnome')
gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
