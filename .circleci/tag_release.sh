#!/bin/bash
set -eu

PACKAGES=(
    $(sed '/^]/{d;q};s/"\(.*\)",\?/\1/;t;d' Cargo.toml)
)

VER=$(sed -ne 's/^version = "\(.*\)".*/\1/;T;p;q' varisat/Cargo.toml)

for PACKAGE in "${PACKAGES[@]}"; do
    PKG_VER=$(sed -ne 's/^version = "\(.*\)".*/\1/;T;p;q' varisat/Cargo.toml)

    if [[ $PKG_VER != $VER ]]; then
        echo "$PACKAGE has version $PKG_VER != $VER" >&2
        exit 1
    fi
done

FILES=(
    varisat/src/lib.rs
    README.md
    varisat/README.md
)

for FILE in "${FILES[@]}"; do
    if ! fgrep -q https://jix.github.io/varisat/manual/$VER/ $FILE; then
        echo "Manual link in $FILE is not up to date" >&2
        exit 1
    fi
done

FILES=(
    README.md
    varisat/README.md
    manual/src/lib/README.md
    manual/src/README.md
)

for FILE in "${FILES[@]}"; do
    if ! fgrep -q https://docs.rs/varisat/$VER/varisat/ $FILE; then
        echo "API docs link in $FILE is not up to date" >&2
        exit 1
    fi
done

if ! fgrep -q "# $VER (" <(head -1 CHANGELOG.md); then
    echo "Changelog entry missing" >&2
    exit 1
fi

if ! git tag -l v$VER | grep -q .; then
    git tag -a v$VER -m "Release of version $VER"
fi

git describe --tags --match='v[0-9]*' --dirty='-d' '--always' | sed 's/^v//'
