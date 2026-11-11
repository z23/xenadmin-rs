#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception
#
# Produces data/xenapi.json for a toolstack release: builds the description generator of
# xapi-project/xen-api (ocaml/idl/json_backend/gen_json.ml) at a tag and runs it. The
# generator writes out the data model that the toolstack of that tag is built from.
#
#   regenerate.sh TAG OUTPUT_DIRECTORY
#
# The copy of this output that upstream keeps at doc/data/xenapi.json is not regenerated
# with each release (at v26.1.11 it was last changed in May 2024 and described nothing
# newer than release 24.10.0), so it is not what this crate uses.
#
# Run it on a throwaway Debian or Ubuntu machine: it installs packages with apt and an
# OCaml package set with opam in the home directory. It changes nothing in this
# repository. Afterwards copy OUTPUT_DIRECTORY/xenapi.json to data/xenapi.json and put
# the tag, the commit and the digest it prints into data/SOURCE; the build refuses a
# description whose digest differs from the one recorded there.
set -euo pipefail
tag="${1:?usage: regenerate.sh TAG OUTPUT_DIRECTORY}"
out="${2:?usage: regenerate.sh TAG OUTPUT_DIRECTORY}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$(mktemp -d)"

export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -qq
sudo apt-get install -y -qq git build-essential m4 pkg-config unzip opam ocaml libgmp-dev

git clone -q --depth 1 --branch "$tag" https://github.com/xapi-project/xen-api "$work/xen-api"
cd "$work/xen-api"
commit="$(git rev-parse HEAD)"

# The distribution's OCaml compiler, so that none is built here.
[ -d "$HOME/.opam" ] || opam init -y --disable-sandboxing --compiler=ocaml-system >/dev/null
eval "$(opam env)"
# What the generator and the libraries it links need from outside the upstream tree.
opam install -y dune yojson fmt xapi-backtrace ptime mtime astring logs unix-errno polly \
  fd-send-recv menhir >/dev/null

generator=ocaml/idl/json_backend/gen_json.exe
dune build "$generator"
test -z "$(git status --porcelain)"   # the source built is the tag's, unmodified
"_build/default/$generator" -destdir "$work/generated" >/dev/null
cp "$work/generated/_data/xenapi.json" "$out/xenapi.json"

echo "repository=xapi-project/xen-api"
echo "tag=$tag"
echo "commit=$commit"
echo "generator=ocaml/idl/json_backend/gen_json.ml"
echo "sha256=$(sha256sum "$out/xenapi.json" | cut -d' ' -f1)"
echo "built with: $(ocamlfind ocamlc -version 2>/dev/null || ocamlc -version) and" \
  "$(opam list --installed --short --columns=name,version dune yojson rpclib ppx_deriving_rpc xapi-backtrace 2>/dev/null | tr -s ' ' | tr '\n' ' ')"
