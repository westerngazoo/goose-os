#!/bin/bash
# goose-upgrade.sh — Pull latest GooseOS kernel and reboot into it
# Put in /root/ on VF2, source from .bashrc for the 'goose' command

GOOSE_DIR="/root/goose-os"

goose() {
    case "${1:-help}" in
        upgrade|up)
            echo "=== GooseOS Upgrade ==="
            cd "$GOOSE_DIR" || { echo "ERROR: $GOOSE_DIR not found"; return 1; }
            echo "Pulling latest..."
            git pull || { echo "ERROR: git pull failed"; return 1; }
            local build=$(cat .build_number 2>/dev/null || echo "?")
            echo "Copying kernel.bin (build $build) to /boot/..."
            cp build/kernel.bin /boot/kernel.bin || { echo "ERROR: cp failed"; return 1; }
            echo ""
            echo "  Build $build ready in /boot/kernel.bin"
            echo "  Run 'goose reboot' to boot into it"
            echo ""
            ;;
        go)
            # upgrade + reboot in one shot
            goose upgrade && goose reboot
            ;;
        reboot|rb)
            echo "Rebooting into GooseOS..."
            sleep 1
            reboot
            ;;
        status|st)
            cd "$GOOSE_DIR" 2>/dev/null || { echo "ERROR: $GOOSE_DIR not found"; return 1; }
            local build=$(cat .build_number 2>/dev/null || echo "?")
            echo "GooseOS build: $build"
            echo "Repo: $GOOSE_DIR"
            git log --oneline -5
            echo ""
            ls -lh /boot/kernel.bin 2>/dev/null || echo "/boot/kernel.bin not found"
            ;;
        *)
            echo "Usage: goose <command>"
            echo ""
            echo "  upgrade (up)   Pull latest kernel and copy to /boot"
            echo "  go             Upgrade + reboot in one shot"
            echo "  reboot  (rb)   Reboot into GooseOS now"
            echo "  status  (st)   Show current build info"
            echo ""
            ;;
    esac
}
