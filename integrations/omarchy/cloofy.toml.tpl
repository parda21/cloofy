# cloofy's colors for the current Omarchy theme. Omarchy renders this file to
# ~/.local/state/omarchy/current/theme/cloofy.toml on every theme change, and cloofy
# follows it while it runs. Edit the copy in ~/.config/omarchy/themed/ to change how every
# theme looks in cloofy, or ship a cloofy.toml with a theme to style just that one.
#
# Shades mix the background toward the text color, so they work for light themes as well
# as dark ones. Any color left out or not rendered keeps the value cloofy derives itself.

# Behind everything, the same as the terminal's.
bg = "{{ background }}"
# The sidebar and the player bar.
surface = "{{ mix background foreground 3% }}"
player = "{{ mix background foreground 2% }}"
# Cards, menus and buttons.
raised = "{{ mix background foreground 7% }}"

text = "{{ foreground }}"
# Artists, counts and other secondary text.
muted = "{{ mix background foreground 60% }}"
# Hints and separators.
faint = "{{ mix background foreground 32% }}"
danger = "{{ red }}"

# A selected row while its list doesn't have focus.
unfocused = "{{ mix background foreground 5% }}"
# Buttons that do nothing right now.
disabled = "{{ mix background foreground 22% }}"
# Where a cover is still loading.
placeholder = "{{ mix background foreground 12% }}"
# The scrollbar's thumb.
thumb = "{{ mix background foreground 40% }}"
# Lyrics around the line being sung fade from near to far.
lyric_near = "{{ mix background foreground 85% }}"
lyric_far = "{{ mix background foreground 25% }}"
# What hovered things lighten toward.
highlight = "{{ bright_foreground }}"

# The accent while no cover sets one, and the "Theme" accent in Settings.
accent = "{{ accent }}"
