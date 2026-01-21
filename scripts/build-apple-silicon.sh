#!/bin/bash
# Build LAO with Apple Silicon optimizations
set -e

echo "🍎 Building LAO for Apple Silicon with optimizations..."
echo ""

# Check we're on Apple Silicon
ARCH=$(uname -m)
if [ "$ARCH" != "arm64" ]; then
    echo "⚠️  Warning: Not running on Apple Silicon (detected: $ARCH)"
    echo "   This build script is optimized for arm64 Macs"
    echo ""
fi

# Verify native Rust toolchain
RUSTC_TARGET=$(rustc -vV | grep host | cut -d' ' -f2)
if [[ "$RUSTC_TARGET" != *"aarch64-apple-darwin"* ]]; then
    echo "❌ Error: Rust toolchain is not native ARM64"
    echo "   Current: $RUSTC_TARGET"
    echo "   Install native toolchain: rustup default stable-aarch64-apple-darwin"
    exit 1
fi

echo "✅ Native ARM64 Rust toolchain detected: $RUSTC_TARGET"
echo ""

# Set optimization flags
export RUSTFLAGS="-C target-cpu=native -C opt-level=3"
export MACOSX_DEPLOYMENT_TARGET="12.0"

echo "🔧 Build settings:"
echo "   RUSTFLAGS=$RUSTFLAGS"
echo "   MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET"
echo ""

# Build plugins with Metal + Accelerate
echo "🔨 Building GGUFPlugin with Metal + Accelerate..."
cd plugins/GGUFPlugin
cargo build --release --features metal,accelerate
cd ../..

echo "🔨 Building LlamaCppPlugin with Metal..."
cd plugins/LlamaCppPlugin
cargo build --release
cd ../..

echo "🔨 Building other plugins..."
cargo build --release --workspace \
    --exclude lao-cli \
    --exclude lao-ui \
    --exclude test_runner

echo "🔨 Building core and CLI..."
cd core
cargo build --release
cd ..

cd cli
cargo build --release
cd ..

# Copy plugins to correct location
echo "📦 Copying plugins to plugins/ directory..."
cp target/release/*.dylib plugins/ 2>/dev/null || true

echo ""
echo "✅ Build complete!"
echo ""
echo "📊 Binary information:"
file target/release/lao-cli
echo ""

echo "🚀 Quick start:"
echo "   cd core && ../target/release/lao-cli plugin-list"
echo "   cd core && ../target/release/lao-cli run ../workflows/test.yaml"
echo ""

echo "💡 Performance tips:"
echo "   • Metal is enabled for GPU acceleration"
echo "   • Accelerate framework is enabled for CPU operations"
echo "   • Use 'n_gpu_layers: 999' for full GPU offload"
echo "   • Monitor with: sudo powermetrics --samplers gpu_power"
echo ""
