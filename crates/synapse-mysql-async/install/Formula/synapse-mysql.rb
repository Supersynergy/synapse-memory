class SynapseMysql < Formula
  desc "Synapse MySQL drop-in — zero-config SQLite-backed MySQL wire server"
  homepage "https://github.com/Supersynergy/synapse"
  version "0.1.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Supersynergy/synapse/releases/download/v#{version}/synapse-mysql-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_REAL_SHA256_AARCH64"
    else
      url "https://github.com/Supersynergy/synapse/releases/download/v#{version}/synapse-mysql-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_REAL_SHA256_X86_64"
    end
  end

  on_linux do
    url "https://github.com/Supersynergy/synapse/releases/download/v#{version}/synapse-mysql-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "REPLACE_WITH_REAL_SHA256_LINUX"
  end

  def install
    bin.install "synapse-mysql-async"
    (var/"synapse").mkpath
  end

  service do
    run [opt_bin/"synapse-mysql-async", "-f", "#{var}/synapse/default.db", "-b", "0.0.0.0:3306"]
    keep_alive true
    log_path var/"log/synapse-mysql.log"
    error_log_path var/"log/synapse-mysql.log"
  end

  def caveats
    <<~EOS
      Synapse MySQL drop-in is listening on port 3306.

      Default credentials:
        user: root
        password: synapse
        host: 127.0.0.1

      Start the service:
        brew services start synapse-mysql

      For WordPress/Drupal/Joomla:
        DB_HOST=127.0.0.1, DB_USER=root, DB_PASSWORD=synapse

      Data dir: #{var}/synapse/
    EOS
  end

  test do
    system "#{bin}/synapse-mysql-async", "--version"
  end
end
