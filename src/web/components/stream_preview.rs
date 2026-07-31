use crate::web::components::ui::button::{button_variants, ButtonSize, ButtonVariant};
use crate::web::components::ui::card::{card, card_content, card_header, card_title};
use topcoat::{
    view::{attributes, component, view},
    Result,
};

#[component]
pub async fn stream_preview() -> Result {
    let publish_button = button_variants(ButtonVariant::Primary, ButtonSize::Md);
    let stop_button = button_variants(ButtonVariant::Destructive, ButtonSize::Md);

    view! {
        card(
            attrs: attributes! { class="mb-8" },
            card_header(
                <div class="flex flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
                    <div>
                        card_title("Staged Stream")
                        <p id="stream-stage-detail" class="mt-1 text-sm text-muted-foreground">
                            "Waiting for an RTMP stream. Nothing will be sent to external targets automatically."
                        </p>
                    </div>
                    <span id="stream-stage-badge" class="mt-2 w-fit rounded-full bg-muted px-3 py-1 text-xs font-semibold uppercase tracking-wide sm:mt-0">
                        "Offline"
                    </span>
                </div>
            )
            card_content(
                <div class="grid gap-6 lg:grid-cols-[minmax(0,2fr)_minmax(16rem,1fr)]">
                    <div class="relative aspect-video overflow-hidden rounded-xl border bg-black">
                        <video
                            id="stream-preview-video"
                            class="h-full w-full bg-black object-contain"
                            controls="controls"
                            autoplay="autoplay"
                            muted="muted"
                            playsinline="playsinline"
                        ></video>
                        <div id="stream-preview-placeholder" class="absolute inset-0 flex items-center justify-center p-6 text-center text-sm text-white/70">
                            "Start streaming to the RTMP ingest to create a preview."
                        </div>
                    </div>
                    <div class="flex flex-col justify-between gap-6 rounded-xl border bg-surface p-5">
                        <div>
                            <h3 class="font-semibold">"Publishing controls"</h3>
                            <p class="mt-2 text-sm leading-relaxed text-muted-foreground">
                                "Review the staged preview, then publish it to every enabled RTMP target. Going-live notifications are sent only when you publish."
                            </p>
                        </div>
                        <div class="flex flex-col gap-3">
                            <button id="publish-staged-stream" type="button" class=(publish_button) disabled="disabled">
                                "Publish Live"
                            </button>
                            <button id="stop-publishing-stream" type="button" class=(stop_button) disabled="disabled">
                                "Stop Publishing"
                            </button>
                            <p id="stream-action-error" class="hidden text-sm text-destructive"></p>
                        </div>
                    </div>
                </div>
                <noscript>
                    <p class="mt-4 text-sm text-destructive">"JavaScript is required for the live HLS preview and publishing controls."</p>
                </noscript>
            )
        )
    }
}
