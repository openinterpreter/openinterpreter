###########################################################################################
# This Dockerfile runs an LMC-compatible websocket server at / on port 8000.              #
# To learn more about LMC, visit https://docs.openinterpreter.com/protocols/lmc-messages. #
###########################################################################################

FROM python:3.11.8

# Set environment variables
# ENV OPENAI_API_KEY ...

ENV HOST 0.0.0.0
# ^ Sets the server host to 0.0.0.0, Required for the server to be accessible outside the container

# Copy required files into container
WORKDIR /app
RUN pip install --no-cache-dir uv
RUN mkdir -p interpreter scripts
COPY interpreter/ interpreter/
COPY scripts/ scripts/
COPY pyproject.toml README.md ./

# Expose port 8000
EXPOSE 8000

# Install server dependencies
RUN uv pip install --system ".[server]"

# Start the server
ENTRYPOINT ["interpreter", "--server"]
