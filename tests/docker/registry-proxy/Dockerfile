FROM openresty/openresty:1.25.3.2-alpine

LABEL org.opencontainers.image.title="Guardian Registry Proxy"
LABEL org.opencontainers.image.description="Routes requests to per-cluster registry backends based on VM source IP"
LABEL org.opencontainers.image.version="1.0.0"

COPY nginx.conf /etc/nginx/conf.d/proxy.conf

EXPOSE 9000

CMD ["nginx", "-g", "daemon off;"]
