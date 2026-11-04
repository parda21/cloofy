# Made for each release from packaging/homebrew/cloofy-full.rb in https://github.com/parda21/cloofy.
# The build with Spotify and YouTube Music. cloofy and cloofy-ytmusic have one each.
class CloofyFull < Formula
  desc "Spotify and YouTube Music client for the terminal"
  homepage "https://github.com/parda21/cloofy"
  version "@VERSION@"
  license "MIT"

  # For YouTube Music: mpv plays, yt-dlp finds the streams, and deno solves YouTube's
  # challenges for yt-dlp.
  depends_on "deno"
  depends_on "mpv"
  depends_on "yt-dlp"

  on_macos do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-full-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_FULL_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-full-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_FULL_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-full-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_FULL_AARCH64_UNKNOWN_LINUX_GNU@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-full-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_FULL_X86_64_UNKNOWN_LINUX_GNU@"
    end
  end

  conflicts_with "cloofy", "cloofy-ytmusic", because: "each installs a cloofy binary"

  def install
    bin.install "cloofy"
  end

  def caveats
    text = <<~EOS
      Spotify needs Premium, and a free Spotify app of your own to read your
      library. Create the app and put its client ID into
      ~/.config/cloofy/config.toml, as https://github.com/parda21/cloofy#setup
      describes. YouTube Music plays without an account; signed in, cloofy also
      shows your library and likes, read from your browser's YouTube cookies,
      which stay on this computer. Then sign in, and check that everything works:
        cloofy login
        cloofy doctor
    EOS
    if OS.linux?
      text += <<~EOS

        Spotify plays through PulseAudio or PipeWire, with your system's ALSA and
        PulseAudio libraries (libasound2 and libpulse0 on Debian and Ubuntu).
      EOS
    end
    text
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cloofy --version")
  end
end
