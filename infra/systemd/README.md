# llama.cpp Setup in infra/systemd

This README provides instructions on how to get `llama.cpp` up and running with any given model using the systemd service.

## Prerequisites
- Ensure you have `llama.cpp` installed in your system.
- Have the necessary dependencies installed.

## Steps to Setup

1. **Install Dependencies**:
   - Install any required dependencies for `llama.cpp`.

2. **Configure the Model**:
   - Set the environment variables for the model in the systemd service file or a drop-in configuration file.
   - Example configuration in `~/.config/systemd/user/llama-server.service.d/override.conf`:
     ```ini
     [Service]
     Environment=LLAMA_MODEL=your/model:your/config
     Environment=LLAMA_HOST=0.0.0.0
     Environment=LLAMA_PORT=8080
     Environment=LLAMA_CTX=16384
     Environment=LLAMA_GPU_LAYERS=999
     ```

3. **Start the Systemd Service**:
   - Reload the systemd daemon to apply changes: `systemctl --user daemon-reload`
   - Start the `llama-server` service: `systemctl --user start llama-server`
   - Enable the service to start on boot: `systemctl --user enable llama-server`

## Verification
- Ensure the service is running: `systemctl --user status llama-server`
- Verify the model is loaded and the service is accessible.

## Troubleshooting
- Check the logs for any errors: `journalctl --user -u llama-server`
- Ensure all environment variables are correctly set. For more information, see [services/coding/README.md](../../services/coding/README.md).

