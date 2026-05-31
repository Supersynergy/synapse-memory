class Synapse < Formula
  desc "Local-first memory and freshness layer for coding agents"
  homepage "https://github.com/Supersynergy/synapse"
  url "https://github.com/Supersynergy/synapse/archive/refs/tags/v1.0.1-rc.1.tar.gz"
  sha256 "REPLACE_WITH_RELEASE_TARBALL_SHA256"
  license "MIT"
  head "https://github.com/Supersynergy/synapse.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "build", "--release", "-p", "synapse-cli", "--bin", "synx"
    system "cargo", "build", "--release", "-p", "synapsed", "--bin", "synapsed", "--bin", "synx-fast"
    bin.install "target/release/synx"
    bin.install "target/release/synapsed"
    bin.install "target/release/synx-fast"
  end

  service do
    run [opt_bin/"synapsed", "--file", var/"synapse/brain.db", "--sock", var/"synapse/synapse.sock", "--lazy-embed"]
    keep_alive true
    working_dir var/"synapse"
    log_path var/"log/synapse.log"
    error_log_path var/"log/synapse.log"
  end

  test do
    assert_match "synx", shell_output("#{bin}/synx --help")
  end
end
