package main

import (
	"context"
	"dagger/ws/internal/dagger"
)

type Ws struct{}

// Build the Rust project in release mode
func (m *Ws) Build(ctx context.Context) (string, error) {
	return m.builder().Stdout(ctx)
}

// Binary returns the compiled ws binary as a downloadable file
func (m *Ws) Binary(ctx context.Context) (*dagger.File, error) {
	return m.builder().File("/src/target/release/ws"), nil
}

// Run cargo test
func (m *Ws) Test(ctx context.Context) (string, error) {
	src := dag.CurrentModule().Source()
	return dag.Container().
		From("rust:latest").
		WithDirectory("/src", src).
		WithWorkdir("/src").
		WithExec([]string{"cargo", "test"}).
		Stdout(ctx)
}

// Lint with cargo clippy
func (m *Ws) Lint(ctx context.Context) (string, error) {
	src := dag.CurrentModule().Source()
	return dag.Container().
		From("rust:latest").
		WithDirectory("/src", src).
		WithWorkdir("/src").
		WithExec([]string{"cargo", "clippy", "--", "-D", "warnings"}).
		Stdout(ctx)
}

// Container builds a minimal deployment image
func (m *Ws) Container(ctx context.Context) (*dagger.Container, error) {
	return m.buildImage(), nil
}

// Publish builds and pushes the image to a registry
func (m *Ws) Publish(ctx context.Context, address string) (string, error) {
	return m.buildImage().Publish(ctx, address)
}

// Load exports the image as a tarball for 'docker load'
func (m *Ws) Load(ctx context.Context) (*dagger.File, error) {
	return m.buildImage().AsTarball(), nil
}

// --- internal helpers ---

func (m *Ws) builder() *dagger.Container {
	src := dag.CurrentModule().Source()
	return dag.Container().
		From("rust:latest").
		WithDirectory("/src", src).
		WithWorkdir("/src").
		WithExec([]string{"cargo", "build", "--release"})
}

func (m *Ws) buildImage() *dagger.Container {
	binary := m.builder().File("/src/target/release/ws")

	return dag.Container().
		From("debian:stable-slim").
		WithExec([]string{
			"apt-get", "update",
		}).
		WithExec([]string{
			"apt-get", "install", "--no-install-recommends", "-y",
			"ca-certificates", "curl",
			"nodejs", "npm",
			"which", "unzip", "xz-utils", "bzip2",
		}).
		WithExec([]string{"apt-get", "clean"}).
		WithExec([]string{"rm", "-rf", "/var/lib/apt/lists/*"}).
		WithFile("/usr/local/bin/ws", binary).
		WithWorkdir("/workspace").
		WithEntrypoint([]string{"ws"}).
		WithDefaultArgs([]string{"--help"})
}
