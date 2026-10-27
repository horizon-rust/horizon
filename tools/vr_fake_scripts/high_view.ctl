# Above the terrain, for runs about distant terrain: the player watches the far view from
# high up (spectator: no fall) instead of whatever is in front of the spawn.
wait-for loaded --timeout 300
command gamemode spectator
command tp @s ~ 220 ~
