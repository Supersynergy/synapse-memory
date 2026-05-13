class Synx < Formula
  desc "Synapse CLI — 8ms hybrid search, 113k docs, SimSIMD kernels"
  homepage "https://github.com/Supersynergy/synapse"
  version "1.0.1-rc.1"

  on_macos do
    on_arm do
      url "https://github.com/Supersynergy/synapse/releases/download/v1.0.1-rc.1/synx-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_AARCH64_APPLE_DARWIN"
    end
    on_intel do
      url "https://github.com/Supersynergy/synapse/releases/download/v1.0.1-rc.1/synx-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/Supersynergy/synapse/releases/download/v1.0.1-rc.1/synx-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64_LINUX_GNU"
    end
  end

  def install
    bin.install "synx"
    # Shell completions (generated at build time, included in tarball)
    bash_completion.install "completions/synx.bash" if File.exist?("completions/synx.bash")
    zsh_completion.install "completions/_synx" if File.exist?("completions/_synx")
    fish_completion.install "completions/synx.fish" if File.exist?("completions/synx.fish")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/synx --version")
  end
end
