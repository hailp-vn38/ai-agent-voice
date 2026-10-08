# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS source
WORKDIR /app
COPY apps/admin-web/package.json apps/admin-web/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY apps/admin-web/ ./

FROM source AS test
COPY docker/smoke.mjs /smoke.mjs
CMD ["sh", "-c", "npm test && npm run build"]

FROM source AS build
ENV VITE_API_BASE_URL="" VITE_ADMIN_TOKEN=""
RUN npm run build

FROM nginx:1.28-bookworm AS production
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 8080
