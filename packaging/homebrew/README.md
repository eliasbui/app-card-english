# Homebrew tap cho Card English

[Card English](https://github.com/eliasbui/app-card-english) là app desktop học từ vựng cho người Việt: thẻ ôn theo FSRS, giải thích bằng AI, chấm phát âm ngay trên máy. Bản macOS chạy trên cả Apple silicon và Intel, từ macOS 11 trở lên.

    brew tap eliasbui/card-english
    brew install --cask eliasbui/card-english/card-english

Ghi đủ tên cask như trên thì Homebrew chỉ tin đúng cask này, không cần `brew trust`. App chỉ ký ad hoc và chưa notarize, nên cask gỡ cờ quarantine sau khi cài để macOS cho mở app.

Cập nhật bằng `brew upgrade --cask card-english`; app tự báo khi có bản mới và nhắc đúng lệnh này. `brew uninstall --cask card-english` gỡ app cùng mục mở cùng máy tính nếu đã bật, thêm `--zap` để xóa cả dữ liệu trong `~/Library/Application Support/CardEnglish`.

Repo này được cập nhật tự động mỗi khi Card English phát hành bản mới, bởi `scripts/update-cask.sh` của repo app.
