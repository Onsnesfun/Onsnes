# Serve the self-contained Onsnes site from nginx.
FROM nginx:alpine

COPY index.html /usr/share/nginx/html/index.html
COPY Onsnes.png /usr/share/nginx/html/Onsnes.png

EXPOSE 80
