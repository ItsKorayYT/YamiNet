#!/bin/sh
set -eu
[ "$#" = 4 ] || { echo 'usage: import-gcp.sh PROJECT BUCKET IMAGE GUEST_BUILD_DIRECTORY' >&2; exit 2; }
PROJECT=$1 BUCKET=$2 IMAGE=$3 BUILD=$4
# GNU tar produces the GCE custom image import format. The local disk is never mounted.
tar --format=oldgnu --sparse --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner \
 -C "$BUILD" -cf - disk.raw | gzip -n >"$BUILD/disk.raw.tar.gz"
gcloud storage cp "$BUILD/disk.raw.tar.gz" "gs://$BUCKET/$IMAGE.tar.gz"
gcloud compute images create "$IMAGE" --project="$PROJECT" \
 --source-uri="gs://$BUCKET/$IMAGE.tar.gz" \
 --guest-os-features=UEFI_COMPATIBLE,TDX_CAPABLE,GVNIC,VIRTIO_SCSI_MULTIQUEUE
