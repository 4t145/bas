.DEFAULT_GOAL := help

NIX_PROFILE_OPTIONS ?=

.PHONY: help install install-basrd install-basrd-service install-basrw

# Installers update the same Nix profile, so keep them sequential even with -j.
.NOTPARALLEL:

help:
	@printf '%s\n' \
		'make install                Install basrd, basrctl and basrw into your Nix profile' \
		'make install-basrd          Install basrd and basrctl' \
		'make install-basrd-service  Install basrd and enable/restart its systemd user service' \
		'make install-basrw          Install basrw with its desktop entry and graphics libraries' \
		'' \
		'Pass Nix options with NIX_PROFILE_OPTIONS, e.g. NIX_PROFILE_OPTIONS="--profile /path/to/profile -L"'

install: install-basrd install-basrw

install-basrd:
	./scripts/install-basrd $(NIX_PROFILE_OPTIONS)

install-basrd-service:
	./scripts/install-basrd --service $(NIX_PROFILE_OPTIONS)

install-basrw:
	./scripts/install-basrw $(NIX_PROFILE_OPTIONS)
