# supervisord-now

A single-container web IDE for authoring [now](https://now.dev.br) workflows and running them as supervised jobs on demand.

> [!Note]
> LLM disclaimer: Most of the code was generated with large language models.

## Installation

```bash
nix-build
docker load < result
```

The imported image will have the tag `supervisord-now:latest`.

## Usage

Initialize a `now.nix` workflow at `./workspace`, mount it on `/workspace`, and publish port 9991:

```bash
mkdir ./workspace
nix run git+https://codeberg.org/now-runner/now -- init ./workspace/now.nix
docker run --rm -p 9991:9991 -v ./workspace:/workspace supervisord-now:latest
```

Open <http://localhost:9991>. The mounted directory is the editor workspace.

Alternatively, you can use a `flake.nix` that exposes a `now` workflow in its output.
