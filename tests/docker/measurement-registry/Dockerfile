FROM python:3.13-slim

LABEL org.opencontainers.image.title="Guardian Measurement Registry"
LABEL org.opencontainers.image.description="Trusted TDX measurement whitelist for Guardian cluster admission"
LABEL org.opencontainers.image.version="2.0.0"

WORKDIR /app

# Copy application
COPY app.py .

# Install dependencies (no requirements.txt needed for simple service)
RUN pip install --no-cache-dir \
    fastapi==0.115.6 \
    "uvicorn[standard]==0.34.0" \
    pydantic==2.10.4

# Create non-root user
RUN adduser --disabled-password --gecos "" appuser && \
    chown -R appuser:appuser /app
USER appuser

EXPOSE 9000

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD python -c "import urllib.request; urllib.request.urlopen('http://localhost:9000/health')"

CMD ["uvicorn", "app:app", "--host", "0.0.0.0", "--port", "9000", "--log-level", "info"]
