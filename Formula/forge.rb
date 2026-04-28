class Forge < Formula
  desc "Cognitive infrastructure for AI agents — persistent memory, intelligent guardrails, self-healing knowledge graph"
  homepage "https://github.com/chaosmaximus/forge"
  version "0.6.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/chaosmaximus/forge/releases/download/v#{version}/forge-#{version}-aarch64-apple-darwin.tar.gz"
      # sha256 will be filled by CI after building release binaries
      sha256 "PLACEHOLDER"
    end
    on_intel do
      url "https://github.com/chaosmaximus/forge/releases/download/v#{version}/forge-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/chaosmaximus/forge/releases/download/v#{version}/forge-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER"
    end
  end

  def install
    bin.install "forge-daemon"
    bin.install "forge-next"
  end

  def post_install
    # Create forge directory
    (var/"forge").mkpath
  end

  def caveats
    <<~EOS
      Forge daemon installed. To start:

        forge-next health

      The daemon auto-starts when you run any forge-next command.
      For always-on service:

        forge-next service install
        forge-next service start

      Quick start:
        forge-next bootstrap        # Import existing Claude Code sessions
        forge-next recall "query"   # Search your memories
        forge-next doctor           # Check system health
    EOS
  end

  test do
    assert_match "forge-next", shell_output("#{bin}/forge-next --help")
  end
end
