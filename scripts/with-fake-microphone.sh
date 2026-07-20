#!/usr/bin/env bash

set -Eeuo pipefail

usage() {
    cat >&2 <<'EOF'
Usage:
  scripts/with-fake-microphone.sh AUDIO_FILE -- COMMAND [ARG...]
  scripts/with-fake-microphone.sh --tts TEXT -- COMMAND [ARG...]

Run COMMAND with a private PipeWire/Pulse virtual microphone and feed AUDIO_FILE
through it in real time. The --tts form creates the audio with eSpeak NG.

Enter Talkdown's dictation mode before the default three-second delay expires.
Override it with TALKDOWN_FAKE_MIC_DELAY; override the source sample rate with
TALKDOWN_FAKE_MIC_RATE. Ready-handshake mode defaults to no additional delay.
TALKDOWN_FAKE_MIC_TAIL controls the real-time silence appended after the audio.

Automated consumers can set TALKDOWN_FAKE_MIC_WAIT_FOR_READY=1 and touch the
path in TALKDOWN_FAKE_MIC_READY_FILE when recording has started. The harness
touches TALKDOWN_FAKE_MIC_DONE_FILE after the audio feeder reaches EOF.
EOF
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'with-fake-microphone: required command not found: %s\n' "$1" >&2
        exit 127
    fi
}

if (( $# < 3 )); then
    usage
    exit 2
fi

require_command pactl
require_command ffmpeg
require_command setsid

harness_root=${XDG_RUNTIME_DIR:-/tmp}
harness_dir=$(mktemp -d "${harness_root%/}/talkdown-fake-mic.XXXXXX")
fifo_path="$harness_dir/input.pcm"
source_name="talkdown_test_$$"
source_rate=${TALKDOWN_FAKE_MIC_RATE:-16000}
wait_for_ready=${TALKDOWN_FAKE_MIC_WAIT_FOR_READY:-0}
ready_timeout=${TALKDOWN_FAKE_MIC_READY_TIMEOUT:-60}
if [[ -n ${TALKDOWN_FAKE_MIC_DELAY+x} ]]; then
    feed_delay=$TALKDOWN_FAKE_MIC_DELAY
elif [[ $wait_for_ready == 1 ]]; then
    feed_delay=0
else
    feed_delay=3
fi
silence_tail=${TALKDOWN_FAKE_MIC_TAIL:-1}
ready_path="$harness_dir/ready"
done_path="$harness_dir/done"
module_id=
child_pid=
feeder_pid=

# shellcheck disable=SC2329 # Reached through the trap-owned cleanup function.
terminate_child_group() {
    if [[ -z ${child_pid:-} ]] || ! kill -0 -- "-$child_pid" 2>/dev/null; then
        return
    fi

    kill -TERM -- "-$child_pid" 2>/dev/null || true
    for _ in {1..20}; do
        if ! kill -0 -- "-$child_pid" 2>/dev/null; then
            break
        fi
        sleep 0.05
    done
    if kill -0 -- "-$child_pid" 2>/dev/null; then
        kill -KILL -- "-$child_pid" 2>/dev/null || true
    fi
    wait "$child_pid" 2>/dev/null || true
}

# shellcheck disable=SC2329 # Invoked indirectly by the EXIT trap below.
cleanup() {
    local status=$?
    trap - EXIT INT TERM

    if [[ -n ${feeder_pid:-} ]] && kill -0 "$feeder_pid" 2>/dev/null; then
        kill "$feeder_pid" 2>/dev/null || true
        wait "$feeder_pid" 2>/dev/null || true
    fi
    terminate_child_group
    if [[ -n ${module_id:-} ]]; then
        pactl unload-module "$module_id" >/dev/null 2>&1 || true
    fi
    if [[ -n ${harness_dir:-} && -d $harness_dir ]]; then
        rm -r -- "$harness_dir"
    fi

    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

if [[ $1 == --tts ]]; then
    require_command espeak-ng
    tts_text=$2
    shift 2
    audio_file="$harness_dir/espeak.wav"
    espeak-ng -D -v en-us -s 150 -w "$audio_file" "$tts_text"
else
    audio_file=$1
    shift
    if [[ ! -f $audio_file ]]; then
        printf 'with-fake-microphone: audio file not found: %s\n' "$audio_file" >&2
        exit 2
    fi
fi

if [[ ${1:-} == -- ]]; then
    shift
fi
if (( $# == 0 )); then
    usage
    exit 2
fi
if [[ ! $source_rate =~ ^[1-9][0-9]*$ ]]; then
    printf 'with-fake-microphone: TALKDOWN_FAKE_MIC_RATE must be a positive integer\n' >&2
    exit 2
fi
if [[ ! $feed_delay =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    printf 'with-fake-microphone: TALKDOWN_FAKE_MIC_DELAY must be a non-negative number\n' >&2
    exit 2
fi
if [[ ! $silence_tail =~ ^[0-9]+([.][0-9]+)?$ ]]; then
    printf 'with-fake-microphone: TALKDOWN_FAKE_MIC_TAIL must be a non-negative number\n' >&2
    exit 2
fi
if [[ $wait_for_ready != 0 && $wait_for_ready != 1 ]]; then
    printf 'with-fake-microphone: TALKDOWN_FAKE_MIC_WAIT_FOR_READY must be 0 or 1\n' >&2
    exit 2
fi
if [[ ! $ready_timeout =~ ^[1-9][0-9]*$ ]]; then
    printf 'with-fake-microphone: TALKDOWN_FAKE_MIC_READY_TIMEOUT must be a positive integer\n' >&2
    exit 2
fi

mkfifo "$fifo_path"
module_id=$(pactl load-module module-pipe-source \
    file="$fifo_path" \
    source_name="$source_name" \
    format=s16le \
    rate="$source_rate" \
    channels=1 \
    channel_map=mono)

printf 'Fake microphone: %s (%s Hz mono)\n' "$source_name" "$source_rate" >&2
printf 'Audio starts in %s second(s); enter dictation mode now.\n' "$feed_delay" >&2

PULSE_SOURCE="$source_name" \
PIPEWIRE_NODE="$source_name" \
TALKDOWN_FAKE_MIC_READY_FILE="$ready_path" \
TALKDOWN_FAKE_MIC_DONE_FILE="$done_path" \
setsid --wait "$@" &
child_pid=$!

if [[ $wait_for_ready == 1 ]]; then
    printf 'Waiting for the child to signal recording readiness…\n' >&2
    ready_deadline=$((SECONDS + ready_timeout))
    while [[ ! -e $ready_path ]]; do
        if ! kill -0 "$child_pid" 2>/dev/null; then
            set +e
            wait "$child_pid"
            child_status=$?
            set -e
            child_pid=
            printf 'with-fake-microphone: child exited before its ready signal (%s)\n' \
                "$child_status" >&2
            if (( child_status == 0 )); then
                child_status=1
            fi
            exit "$child_status"
        fi
        if (( SECONDS >= ready_deadline )); then
            printf 'with-fake-microphone: timed out waiting for the ready signal\n' >&2
            exit 124
        fi
        sleep 0.05
    done
fi

sleep "$feed_delay"
(
    exec 3>"$fifo_path"
    ffmpeg -nostdin -hide_banner -loglevel error -re \
        -i "$audio_file" \
        -map_metadata -1 -vn -ac 1 -ar "$source_rate" -f s16le -y pipe:3
    if [[ $silence_tail != 0 ]]; then
        ffmpeg -nostdin -hide_banner -loglevel error -re \
            -f lavfi -i "anullsrc=r=${source_rate}:cl=mono" -t "$silence_tail" \
            -ac 1 -ar "$source_rate" -f s16le -y pipe:3
    fi
) &
feeder_pid=$!

set +e
wait "$feeder_pid"
feeder_status=$?
feeder_pid=
if (( feeder_status != 0 )); then
    printf 'with-fake-microphone: audio feeder failed (%s)\n' "$feeder_status" >&2
    exit "$feeder_status"
fi

touch "$done_path"
printf 'Audio finished; release dictation when the final words appear.\n' >&2
wait "$child_pid"
child_status=$?
child_pid=
exit "$child_status"
