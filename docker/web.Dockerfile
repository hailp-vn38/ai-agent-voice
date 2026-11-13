# syntax=docker/dockerfile:1
FROM node:22-bookworm-slim AS source
WORKDIR /app
COPY package.json package-lock.json ./
COPY apps/admin-web/package.json apps/admin-web/package.json
COPY apps/wiki-desktop/package.json apps/wiki-desktop/package.json
COPY packages/knowledge-ui/package.json packages/knowledge-ui/package.json
RUN --mount=type=cache,target=/root/.npm npm ci --no-audit --no-fund
COPY apps/admin-web/ apps/admin-web/
COPY packages/knowledge-ui/ packages/knowledge-ui/
WORKDIR /app/apps/admin-web

FROM source AS test
COPY docker/smoke.mjs /smoke.mjs
CMD ["sh", "-c", "npm test && npm run build"]

FROM source AS build
ENV VITE_API_BASE_URL=""
RUN npm run build

FROM nginx:1.28-bookworm AS production
COPY docker/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/apps/admin-web/dist /usr/share/nginx/html
EXPOSE 8080
