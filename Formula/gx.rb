class Gx < Formula
  desc "Brain-first programming language for building transparent AI assistants"
  homepage "https://github.com/elgrhy/gx"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/elgrhy/gx/releases/download/v0.1.0/gx-macos-arm64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_ARM64"
    end
    on_intel do
      url "https://github.com/elgrhy/gx/releases/download/v0.1.0/gx-macos-x64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_X64"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/elgrhy/gx/releases/download/v0.1.0/gx-linux-arm64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_LINUX_ARM64"
    end
    on_intel do
      url "https://github.com/elgrhy/gx/releases/download/v0.1.0/gx-linux-x64.tar.gz"
      sha256 "REPLACE_WITH_ACTUAL_SHA256_LINUX_X64"
    end
  end

  def install
    bin.install "gx"
  end

  def caveats
    <<~EOS
      GX Language is installed. Quick start:

        gx help
        gx init my-agent
        cd my-agent && gx run main.gx

      To use AI features, set your API key:
        export OPENAI_API_KEY=sk-...
        export ANTHROPIC_API_KEY=sk-ant-...

      For local AI (no cloud):
        brew install ollama && ollama serve
        ollama pull llama3
    EOS
  end

  test do
    assert_match "gx #{version}", shell_output("#{bin}/gx version")

    (testpath/"hello.gx").write <<~GX
      agent "test" {
        when started { say "ok" }
        brain { plan {} execute {} remember {} communicate {} }
      }
    GX
    assert_match "ok", shell_output("#{bin}/gx run hello.gx")
  end
end
