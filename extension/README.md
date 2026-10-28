# Tiện ích Card English cho trình duyệt

Bôi đen một từ trên bất kỳ trang web nào, bấm chuột phải và chọn **Thêm vào Card English**. Từ đó, cùng câu chứa nó và
địa chỉ trang, được gửi tới app Card English trên máy: app mở ra và tra từ như khi dùng phím tắt thêm từ. Nếu app
chưa mở, từ được giữ lại và hiện ra ở lần mở app tiếp theo.

Dùng được với Chrome, Edge, Brave và Cốc Cốc.

## Cài đặt

1. Mở app Card English ít nhất một lần. Mỗi lần mở, app tự đăng ký với các trình duyệt đã cài trên máy để tiện ích
   nói chuyện được với app.
2. Mở trang quản lý tiện ích của trình duyệt:
   - Chrome: `chrome://extensions`
   - Edge: `edge://extensions`
   - Brave: `brave://extensions`
   - Cốc Cốc: `coccoc://extensions`
3. Bật **Chế độ dành cho nhà phát triển** (Developer mode).
4. Bấm **Tải tiện ích đã giải nén** (Load unpacked) và chọn thư mục `extension` này.
5. Mở lại trình duyệt nếu tiện ích báo chưa kết nối được với Card English.

Tiện ích luôn có cùng một mã (ID) `bnalkdmmggdignneaebhplodflimgiip`, nhờ khóa công khai ghi trong `manifest.json`,
nên app nhận ra nó dù bạn tải lại thư mục ở chỗ khác.

## Quyền tiện ích dùng

- Menu chuột phải, và đọc câu quanh từ đã bôi đen trên trang bạn vừa bấm (chỉ trang đó, chỉ lúc bấm).
- Gửi tin nhắn tới app Card English trên máy. Không có gì được gửi lên mạng.
