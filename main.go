package main

import (
	"context"
	"dagger/ws/internal/dagger"
)

type Ws struct{}

// Container builds a minimal deployment image
func (m *Ws) Container(ctx context.Context) (*dagger.Container, error) {
	return m.buildImage(), nil
}

// Publish builds and pushes the image to a registry
func (m *Ws) Publish(ctx context.Context, address string) (string, error) {
	return m.buildImage().Publish(ctx, address)
}

// Harness builds the ws-pi npm package.
func (m *Ws) Harness() *dagger.Container {
	return dag.Container().
		From("node:22-bookworm-slim").
		WithDirectory("/app", dag.CurrentModule().Source().Directory("harness")).
		WithWorkdir("/app").
		WithExec([]string{"npm", "ci"}).
		WithExec([]string{"npm", "run", "build"})
}

// PublishHarness publishes ws-pi to npm. npmToken is never written to the source tree.
func (m *Ws) PublishHarness(ctx context.Context, npmToken *dagger.Secret) (string, error) {
	return m.Harness().
		WithSecretVariable("NPM_TOKEN", npmToken).
		WithExec([]string{"sh", "-ec", "trap 'rm -f .npmrc' EXIT; printf '%s\\n' \"//registry.npmjs.org/:_authToken=${NPM_TOKEN}\" > .npmrc; npm publish --access public"}).
		Stdout(ctx)
}

// --- internal helpers ---

func (m *Ws) buildImage() *dagger.Container {
	src := dag.CurrentModule().Source()
	builder := dag.Container().
		From("rust:latest").
		WithDirectory("/src", src).
		WithWorkdir("/src").
		WithExec([]string{"cargo", "build", "--release"})
	binary := builder.File("/src/target/release/ws")

	return dag.Container().
		From("debian:stable-slim").
		WithExec([]string{"apt-get", "update"}).
		WithExec([]string{
			"apt-get", "install", "--no-install-recommends", "-y",
			"ca-certificates", "curl", "git",
			"which", "unzip", "xz-utils", "bzip2",
		}).
		WithExec([]string{"sh", "-c", "useradd -u 1000 -m -d /home/kl kl && chown -R 1000:1000 /home/kl"}).
		WithExec([]string{"apt-get", "clean"}).
		WithExec([]string{"rm", "-rf", "/var/lib/apt/lists/*"}).
		WithFile("/usr/local/bin/ws", binary).
		WithWorkdir("/workspace").
		WithEntrypoint([]string{"ws"}).
		WithDefaultArgs([]string{"--help"})
}
