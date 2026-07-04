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
			"ca-certificates", "curl",
			"which", "unzip", "xz-utils", "bzip2",
		}).
		WithExec([]string{"apt-get", "clean"}).
		WithExec([]string{"rm", "-rf", "/var/lib/apt/lists/*"}).
		WithFile("/usr/local/bin/ws", binary).
		WithWorkdir("/workspace").
		WithEntrypoint([]string{"ws"}).
		WithDefaultArgs([]string{"--help"})
}
