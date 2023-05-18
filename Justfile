# just manual: https://github.com/casey/just#readme

_default:
  just --list

docker:
  #!/bin/bash -eux
  nix build .#dockerImage
  ./result | docker load

