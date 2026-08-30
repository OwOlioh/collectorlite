import { useState } from "react";

interface CoverImageProps {
  src?: string | null;
  alt?: string;
}

/**
 * Lazy-loaded cover with a blur-up style skeleton.
 * Shows a shimmer placeholder until the image decodes, then fades it in.
 * Falls back to a "无封面" placeholder if there is no source or it errors.
 */
export function CoverImage({ src, alt = "" }: CoverImageProps) {
  const [loaded, setLoaded] = useState(false);
  const [errored, setErrored] = useState(false);

  if (!src || errored) {
    return <div className="cover-placeholder">无封面</div>;
  }

  return (
    <>
      {!loaded && <div className="cover-shimmer" aria-hidden="true" />}
      <img
        className={`cover-img${loaded ? " is-loaded" : ""}`}
        src={src}
        alt={alt}
        loading="lazy"
        decoding="async"
        onLoad={() => setLoaded(true)}
        onError={() => setErrored(true)}
      />
    </>
  );
}
