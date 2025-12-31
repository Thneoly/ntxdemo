# 在 Ubuntu 上搭建 Harbor

参考文档：https://goharbor.io/docs/2.14.0/

## 1. 安装依赖

```bash
sudo apt update
sudo apt install -y apt-transport-https ca-certificates curl software-properties-common
```

## 2. 安装 Docker 和 Docker Compose

Harbor 需要 Docker 和 Docker Compose 来运行，所以需要先安装它们。

### 2.1 安装 Docker

```bash
sudo apt install -y docker.io
sudo systemctl start docker
sudo systemctl enable docker
docker --version
```

### 2.2 安装 Docker Compose

Docker Compose 用于管理多个容器。

说明：本文后续示例以 `docker-compose` 为主；如果你使用的是 Docker Compose V2 插件，请将 `docker-compose` 替换为 `docker compose`。

```bash
sudo curl -L "https://github.com/docker/compose/releases/download/1.29.2/docker-compose-$(uname -s)-$(uname -m)" \
	-o /usr/local/bin/docker-compose
sudo chmod +x /usr/local/bin/docker-compose
docker-compose --version
```

## 3. 下载并配置 Harbor

Harbor 官方提供了安装包，可以通过 `docker-compose` 启动。

### 3.1 下载 Harbor 安装包

```bash
mkdir -p ~/harbor
cd ~/harbor

curl -L https://github.com/goharbor/harbor/releases/download/v2.14.0/harbor-offline-installer-v2.14.0.tgz -o harbor-offline-installer.tgz

tar xvf harbor-offline-installer.tgz
```

### 3.2 配置 Harbor

解压后会得到 `harbor/` 目录，其中包含 `harbor.yml` 配置文件。

```bash
cd harbor
cp harbor.yml.tmpl harbor.yml
vim harbor.yml
```

常见需要调整的配置项示例：

```yaml
hostname: 192.168.1.100

# 如果需要启用 HTTPS，配置证书相关项；不需要可跳过 https 段。

# data_volume: 配置 Harbor 存储卷的位置，默认是 /data
```

完成配置后保存并退出。

## 4. 启动 Harbor

```bash
sudo ./install.sh
```

## 5. 访问 Harbor

安装和启动完成后，通过浏览器访问：

```text
http://<your-server-ip>:80
```

默认用户名：`admin`

默认密码：`Harbor12345`

## 6. 配置防火墙（可选）

如果 Ubuntu 启用了防火墙，需要放通 80 端口与 443 端口（如启用 HTTPS）：

```bash
sudo ufw allow 80,443/tcp
sudo ufw reload
```

## 7. 配置 Docker 客户端连接 Harbor

登录 Harbor：

```bash
docker login <harbor-host>
```

### 7.1（可选）使用 HTTP（不启用 HTTPS）连接 Harbor

如果你的 Harbor 暂时只提供 HTTP（没有 TLS），Docker 默认会按 HTTPS 连接，通常会报错。此时需要把 Harbor 加到 Docker 的 `insecure-registries`。

1) 配置 Docker `daemon.json`

```bash
sudo mkdir -p /etc/docker
sudo vim /etc/docker/daemon.json
```

写入（或合并）如下内容（地址必须与你后续使用的仓库地址一致；如果你访问用的是 80 端口，建议写成 `<harbor-host>:80`）：

```json
{
	"insecure-registries": [
		"<harbor-host>:80"
	]
}
```

2) 重启 Docker

```bash
sudo systemctl restart docker
```

3) 重新登录验证

```bash
docker login <harbor-host>:80
```

安全提示：`insecure-registries` 会允许明文传输与跳过证书校验，仅建议用于内网/开发环境；生产环境建议开启 HTTPS。

## 8. （可选）开启 HTTPS（从生成证书开始）

说明：生产环境建议使用企业 CA / ACME（如 Let's Encrypt）签发的证书。下面给的是“自签证书”最小流程，便于内网/开发环境快速跑通。

### 8.1 生成自签证书（包含 SAN）

将 `<harbor-host>` 替换为你实际访问 Harbor 的域名或 IP（建议用域名；若使用 IP，SAN 里也必须包含该 IP）。

```bash
export HARBOR_HOST=<harbor-host>
export HARBOR_IP=<harbor-ip>
mkdir -p ~/harbor/certs
cd ~/harbor/certs

# 生成私钥
openssl genrsa -out harbor.key 4096

# 生成自签证书（关键：-addext subjectAltName）
openssl req -x509 -new -nodes \
	-key harbor.key \
	-sha256 -days 3650 \
	-out harbor.crt \
	-subj "/CN=${HARBOR_HOST}" \
	-addext "subjectAltName=DNS:${HARBOR_HOST},IP:${HARBOR_IP}"
```

说明：

- 如果你用 **IP** 访问/登录（例如 `docker login 192.168.31.138`），证书 SAN 必须包含 `IP:192.168.31.138`，否则 Docker 会报：
	- `x509: cannot validate certificate for 192.168.31.138 because it doesn't contain any IP SANs`
- 如果你只用 **域名** 访问/登录，可以不填 `HARBOR_IP`，并仅保留 `DNS:<domain>`。

如果你需要同时支持多个域名/或 IP，可扩展 SAN，例如：

```bash
openssl req -x509 -new -nodes \
	-key harbor.key \
	-sha256 -days 3650 \
	-out harbor.crt \
	-subj "/CN=${HARBOR_HOST}" \
	-addext "subjectAltName=DNS:${HARBOR_HOST},DNS:harbor.local,IP:${HARBOR_IP},IP:192.168.1.100"
```

生成后可快速检查 SAN：

```bash
openssl x509 -in ~/harbor/certs/harbor.crt -noout -text | sed -n '/Subject Alternative Name/,+2p'
```

### 8.2 配置 Harbor 使用 HTTPS

编辑 `harbor.yml`，确保 `hostname` 与证书一致，并开启 `https` 段：

```bash
cd ~/harbor/harbor
vim harbor.yml
```

示例（按实际路径调整）：

```yaml
hostname: <harbor-host>

https:
  port: 443
  certificate: /home/<user>/harbor/certs/harbor.crt
  private_key: /home/<user>/harbor/certs/harbor.key
```

修改后重新生成配置并启动（离线安装包常见流程）：

```bash
cd ~/harbor/harbor
sudo ./prepare
sudo docker-compose down
sudo docker-compose up -d 
```

### 8.3 让客户端信任 Harbor 证书（Docker / ORAS）

如果你使用的是 **自签 HTTPS**，客户端侧必须信任该证书/CA，否则常见报 `x509: certificate signed by unknown authority`。

说明：

- Docker（daemon）主要读取 `/etc/docker/certs.d/.../ca.crt`
- ORAS（CLI）默认走系统信任链；如是自签，需要使用 `--ca-file` 或把 CA 装进系统信任库

#### 8.3.1 Docker：配置 `/etc/docker/certs.d`

```bash
sudo mkdir -p /etc/docker/certs.d/<harbor-host>
sudo cp ~/harbor/certs/harbor.crt /etc/docker/certs.d/<harbor-host>/ca.crt
sudo systemctl restart docker
```

如果你把 Harbor 暴露在非 443 端口（例如 `8443`），Docker 的信任目录需要带端口：

```bash
sudo mkdir -p /etc/docker/certs.d/<harbor-host>:8443
sudo cp ~/harbor/certs/harbor.crt /etc/docker/certs.d/<harbor-host>:8443/ca.crt
sudo systemctl restart docker
```

#### 8.3.2 ORAS：使用 `--ca-file` 或安装到系统 CA

方式 A（推荐，最简单）：在 ORAS 命令里显式指定 CA 文件：

```bash
oras login --ca-file ~/harbor/certs/harbor.crt <harbor-host>
```

方式 B（全局生效）：把 CA 安装进系统信任库（Ubuntu）：

```bash
sudo install -m 0644 ~/harbor/certs/harbor.crt /usr/local/share/ca-certificates/harbor-ca.crt
sudo update-ca-certificates
```

访问地址改为：

```text
https://<harbor-host>:443
```

### 8.4 HTTPS 开启后 `docker login` 失败（EOF / x509）如何排查

你遇到的错误：

- `Error response from daemon: Get "https://<harbor-host>/v2/": EOF`

通常表示 **TLS 握手阶段连接被对端关闭**（例如 443 上不是 HTTPS、Harbor proxy/nginx 没起来、证书/私钥加载失败、端口/转发异常）。

#### 8.4.1 客户端快速验证（先确认 443 上到底是不是 HTTPS）

1) 看端口是否可达：

```bash
nc -vz <harbor-host> 443
```

2) 直接用 curl 看 `v2` 端点（期望是 `401 Unauthorized` 或带 `WWW-Authenticate` 的响应；如果握手失败/被断开，会更直观）：

```bash
curl -vk https://<harbor-host>/v2/
```

3) 看 TLS 握手和证书链（能拿到证书就说明 443 上在说 TLS）：

```bash
openssl s_client -connect <harbor-host>:443 -servername <harbor-host> -showcerts </dev/null
```

#### 8.4.2 Docker 侧排查（证书信任 & 访问地址一致）

1) **确保证书 SAN 覆盖你实际使用的地址**：

- 如果你用 IP 登录（如 `docker login 192.168.31.138`），证书的 SAN 里需要包含 `IP:192.168.31.138`
- 如果证书只包含域名（DNS SAN），请用域名登录（如 `docker login harbor.example.com`）

2) **信任自签 CA**（自签证书必做，否则常见报 `x509: certificate signed by unknown authority`）：

```bash
sudo mkdir -p /etc/docker/certs.d/<harbor-host>
sudo cp <path-to-harbor.crt> /etc/docker/certs.d/<harbor-host>/ca.crt
sudo systemctl restart docker
```

如果你是非 443 端口（例如 `8443`），目录必须写成 `<harbor-host>:8443`。

3) 重试登录（明确端口也有助于排除解析/转发问题）：

```bash
docker login <harbor-host>
# 或
docker login <harbor-host>:443
```

#### 8.4.3 Harbor 侧排查（重点看 proxy/nginx 是否启动成功）

在 Harbor 安装目录执行：

```bash
cd ~/harbor/harbor
sudo docker-compose ps
```

重点关注 `proxy`（nginx）容器是否 `Up`。然后看日志：

```bash
sudo docker-compose logs --tail=200 proxy
sudo docker-compose logs --tail=200 core
```

常见根因与现象：

- 证书/私钥路径写错或文件不可读：`proxy` 容器启动失败或反复重启
- 修改了 `harbor.yml` 但没跑 `./prepare`：配置未生效，443 仍然不是你期望的 TLS 配置
- 防火墙/安全组未放通 443：`curl -vk` 直接超时或无法连接

如果 `proxy` 容器不在 `Up` 状态，优先修复其启动失败原因（通常就是证书路径/权限/文件格式）。

## 9. Harbor 卸载流程（清理容器与数据）

注意：以下步骤会停止 Harbor 并可能删除镜像/数据卷与存储数据；请先确认是否需要备份。

### 9.1 停止并删除 Harbor 容器

```bash
cd ~/harbor/harbor
sudo docker-compose down -v
```

### 9.2 删除 Harbor 配置目录（可选）

```bash
rm -rf ~/harbor
```

### 9.3 删除 Harbor 数据目录（可选，默认可能在 /data）

如果你的 `harbor.yml` 使用默认 `data_volume: /data`，可按需清理：

```bash
sudo rm -rf /data
```

### 9.4（可选）清理残留镜像

```bash
docker images | grep -E "goharbor/|harbor" || true
```

## 10. Harbor 常用指令（运维）

以下命令在安装目录（包含 `docker-compose.yml` 的目录）执行：

```bash
cd ~/harbor/harbor
```

查看容器状态：

```bash
sudo docker-compose ps
```

启动/停止/重启：

```bash
sudo docker-compose up -d
sudo docker-compose stop
sudo docker-compose restart
```

查看日志（全部组件/单个组件）：

```bash
sudo docker-compose logs -f
sudo docker-compose logs -f core
```

重新生成配置（修改 `harbor.yml` 后常用）：

```bash
sudo ./install.sh
```

## 11. 上传 WASM 到 Harbor（OCI Artifact / ORAS）

说明：这里把 WASM component 作为 OCI Artifact 的一个 layer 上传到 Harbor。该方式与镜像仓库一致，便于平台拉取与按 tag 管理。

### 11.0 使用 `wasm-to-oci`（可选）

`wasm-to-oci` 是一个把 `.wasm` 打包成 **OCI Artifact** 并 push 到 registry 的 CLI（Harbor 2.x 支持）。

特点：

- 直接 `push <wasm> <registry/repo:tag>`，不需要你手写 layer media type
- 会使用 `~/.docker/config.json` 里的登录凭据：先 `docker login <harbor-host>` 再执行 `wasm-to-oci`

安装：从 Releases 下载对应平台二进制并放入 PATH（示例为 Linux）：

```bash
# 从 https://github.com/engineerd/wasm-to-oci/releases 下载 linux-amd64-wasm-to-oci
mv linux-amd64-wasm-to-oci wasm-to-oci
chmod +x wasm-to-oci
sudo cp wasm-to-oci /usr/local/bin/
```

push 示例：

```bash
docker login <harbor-host>
wasm-to-oci push ./component.wasm <harbor-host>/<project>/<repo>:<version>
```

pull 示例：

```bash
wasm-to-oci pull <harbor-host>/<project>/<repo>:<version> --out ./component.wasm
```

注意：

- 如果 Harbor 使用 **自签 HTTPS**，请先按本文 `8.3` 配置 Docker 信任证书，否则可能出现 TLS 相关错误。
- 如果你要传 `.wasm` 以外的额外元数据（例如 `actions-catalog.json` 双 layer），请继续使用下面的 ORAS 方式。

### 11.1 安装 ORAS（示例）

如果系统未安装 `oras`，可先安装（示例用 GitHub Release；也可用发行版包管理器）：

```bash
# 推荐：安装官方 Release 二进制（Ubuntu / Linux）
# 说明：如为 arm64，请把 linux_amd64 改成 linux_arm64

ORAS_VERSION=1.3.0
curl -LO "https://github.com/oras-project/oras/releases/download/v${ORAS_VERSION}/oras_${ORAS_VERSION}_linux_amd64.tar.gz"

mkdir -p /tmp/oras-install
tar -zxf "oras_${ORAS_VERSION}_linux_amd64.tar.gz" -C /tmp/oras-install
sudo install -m 0755 /tmp/oras-install/oras /usr/local/bin/oras

# 验证
oras version
```

### 11.2 构建 WASM（以 actions-executor 为例）

在 repo 根目录构建：

```bash
cargo build -p actions-executor --target wasm32-wasip2
```

默认产物路径通常为：

```text
target/wasm32-wasip2/debug/actions_executor.wasm
```

### 11.3 登录 Harbor

```bash
# 需要关闭 代理
oras login <harbor-host>

# 如果 Harbor 使用自签 HTTPS：
oras login --ca-file ~/harbor/certs/harbor.crt  -u admin <harbor-host>

# 仅用于临时排查（跳过证书校验，不建议长期使用）：
oras login --insecure <harbor-host>
```

### 11.4 push：上传 WASM（单 layer）

```bash
oras push <harbor-host>/<project>/<repo>:<version> \
	--artifact-type application/vnd.ntx.action-executor.v1 \
	component.wasm:application/wasm
# oras push --ca-file=/home/cc/Desktop/harbor/certs/harbor.crt   192.168.31.138/ntx/executor:v0.0.1   --artifact-type application/vnd.ntx.action-executor.v1   eventbus.wasm:application/wasm
```

其中 `component.wasm` 需要是你本地要上传的文件名；你可以先复制/重命名：

```bash
cp target/wasm32-wasip2/debug/actions_executor.wasm ./component.wasm
```

### 11.5（可选）push：WASM + Catalog 双 layer

如果你希望 artifact 内同时携带 `actions-catalog.json`：

```bash
oras push <harbor-host>/<project>/<repo>:<version> \
	--artifact-type application/vnd.ntx.action-executor.v1 \
	component.wasm:application/wasm \
	actions-catalog.json:application/json
```

### 11.6 pull：拉取并落盘

```bash
oras pull <harbor-host>/<project>/<repo>:<version> -o <out-dir>
```