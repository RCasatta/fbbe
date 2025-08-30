FROM rust:1.85.0

WORKDIR /fbbe

# Install system dependencies required for building native dependencies
RUN apt-get update && apt-get install -y \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

# Set environment variables needed for librocksdb-sys
ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

COPY ./ ./

RUN cargo install --path .

EXPOSE 3000

CMD ["fbbe"]
