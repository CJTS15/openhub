# Translations: `meson compile -C _build openhub-pot` refreshes the
# template, openhub.pot; add a language code to LINGUAS with its .po file.
i18n.gettext(
  meson.project_name(),
  preset: 'glib',
  args: [
    '--keyword=gettext_f',
    '--keyword=ngettext_f:1,2',
    '--keyword=gettext_noop',
  ],
)
