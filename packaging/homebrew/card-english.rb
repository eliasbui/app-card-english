cask "card-english" do
  version "0.1.0"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/eliasbui/app-card-english/releases/download/v#{version}/CardEnglish-#{version}-macos.zip"
  name "Card English"
  desc "Flashcards with spaced repetition, AI explanations and pronunciation scoring"
  homepage "https://github.com/eliasbui/app-card-english"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on :macos

  app "Card English.app"

  # Signed ad hoc and not notarized: without this Gatekeeper refuses to open it.
  postflight_steps do
    if_path_exists "{{appdir}}/Card English.app" do
      run "/usr/bin/xattr",
          args: ["-dr", "com.apple.quarantine", "{{appdir}}/Card English.app"]
    end
  end

  # The login item that "Mở cùng máy tính" adds goes with the app.
  uninstall launchctl: "com.eliasbui.cardenglish",
            quit:      "com.eliasbui.cardenglish"

  zap trash: "~/Library/Application Support/CardEnglish"
end
