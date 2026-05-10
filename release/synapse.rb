# Homebrew formula draft for Synapse — single-binary vector+lex+graph+CRDT DB.
# Tap publish: `brew tap supersynergy/synapse`, then `brew install synapse`.
# Source build via cargo to avoid prebuilt-binary trust chain.
class Synapse < Formula
  desc "Single-binary vector+lex+graph+CRDT DB. 107×–892× faster than LanceDB/Qdrant @ 10–20k docs"
  homepage "https://github.com/supersynergy/synapse"
  url "https://github.com/supersynergy/synapse/archive/refs/tags/v1.0.1.tar.gz"
  sha256 "PLACEHOLDER_REPLACE_ON_TAG"
  license "Apache-2.0"
  head "https://github.com/supersynergy/synapse.git", branch: "main"

  depends_on "rust" => :build
  depends_on "cmake" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/synapse-cli")
    system "cargo", "install", *std_cargo_args(path: "crates/synapsed")
    bin.install_symlink "synapse" => "synx"
  end

  service do
    run [opt_bin/"synapsed", "--socket", "/tmp/synapse.sock"]
    keep_alive true
    log_path var/"log/synapse.log"
    error_log_path var/"log/synapse.err"
  end

  test do
    system bin/"synx", "ping"
  end
end
