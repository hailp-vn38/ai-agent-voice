# Thiết lập GitHub deploy key cho Wiki sync

Hướng dẫn này cấu hình `vela-server` để đồng bộ vault Wiki với một GitHub repository riêng. Deploy key có quyền ghi chỉ nên được gắn vào repository chứa vault cần đồng bộ.

## Tạo deploy key

Từ thư mục gốc repository trên host, tạo khóa Ed25519 không có passphrase để server không cần tương tác khi chạy:

```sh
ssh-keygen -t ed25519 -f docker/wiki-deploy-key -N "" -C "vela-server-wiki-sync"
chmod 600 docker/wiki-deploy-key
```

Mở GitHub repository → **Settings → Deploy keys → Add deploy key**. Dán nội dung `docker/wiki-deploy-key.pub`, đặt tên nhận diện được, và bật **Allow write access**. Chỉ thêm khóa công khai lên GitHub; không chia sẻ khóa riêng. Mỗi deploy key chỉ nên phục vụ một repository.

## Ghim host key của GitHub

Lấy danh sách host key qua kết nối mạng đáng tin cậy, sau đó kiểm tra fingerprint Ed25519 với trang [GitHub's SSH key fingerprints](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints) trước khi dùng:

```sh
ssh-keyscan -t ed25519 github.com > docker/github-known-hosts
ssh-keygen -lf docker/github-known-hosts
chmod 644 docker/github-known-hosts
```

Fingerprint Ed25519 hiện GitHub công bố là `SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU`. Nếu kết quả không khớp, không dùng file vừa lấy; xác minh lại host/network và nguồn fingerprint.

Hai file `docker/wiki-deploy-key` và `docker/github-known-hosts` bị Git ignore và được Compose mount read-only. Tạo chúng trước khi khởi động production stack; `create_host_path: false` khiến Compose báo lỗi rõ ràng nếu file bị thiếu, thay vì tạo thư mục cùng tên. Khóa riêng phải đọc được bởi user của container (mặc định UID `1000`, hoặc `APP_UID`) và có mode `0600`; trên Linux đặt owner tương ứng, ví dụ `chown 1000:1000 docker/wiki-deploy-key`. Host keys dùng mode `0644`.

## Bật cấu hình

Trong `docker/config.prod.toml`, cấu hình URL SSH của đúng repository:

```toml
[wiki.sync]
enabled = true
remote_url = "git@github.com:<owner>/<repo>.git"
branch = "main"
ssh_key_path = "/app/docker/wiki-deploy-key"
known_hosts_path = "/app/docker/github-known-hosts"
author_name = "Vela Server"
author_email = "vela@localhost"
interval_min = 5
debounce_sec = 30
max_file_mb = 50
```

Đảm bảo `[wiki]` cũng bật và `vault_dir` trỏ tới vault cần đồng bộ. Sau khi thay đổi config hoặc khóa, khởi động lại server để áp dụng. URL remote không được chứa token hoặc mật khẩu.

Để xác nhận runtime image có Git, OpenSSH và có thể đẩy vào bare repository mà không cần mạng, chạy:

```sh
docker compose -f compose.test.yaml --profile checks run --rm git-sync-smoke
```

Service chạy dưới UID `1000`, bỏ `HOME` trong lúc thực hiện lệnh git, và xóa repository thử nghiệm sau khi hoàn tất.
