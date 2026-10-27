# Made for each release from packaging/homebrew/cloofy.rb in https://github.com/parda21/cloofy.
class Cloofy < Formula
  desc "Spotify client for the terminal"
  homepage "https://github.com/parda21/cloofy"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_AARCH64_UNKNOWN_LINUX_GNU@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_X86_64_UNKNOWN_LINUX_GNU@"
    end
  end

  def install
    bin.install "cloofy"
  end

  def caveats
    text = <<~EOS
      cloofy needs Spotify Premium, and a free Spotify app of your own to read
      your library. Create the app and put its client ID into
      ~/.config/cloofy/config.toml, as https://github.com/parda21/cloofy#setup
      describes. Then sign in, and check that everything works:
        cloofy login
        cloofy doctor
    EOS
    if OS.linux?
      text += <<~EOS

        cloofy plays through PulseAudio or PipeWire, with your system's ALSA and
        PulseAudio libraries (libasound2 and libpulse0 on Debian and Ubuntu).
      EOS
    end
    text
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cloofy --version")
  end
end
