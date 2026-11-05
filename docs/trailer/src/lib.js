const BEAT = {{BEAT}};
const DUR = {{DUR}};
// b(n): the time of beat n inside this scene, snapped to a 30 fps frame.
const b = (n) => Math.round(n * BEAT * 30) / 30;
const tl = gsap.timeline({ paused: true });
// Hold the scene's last frame until its slot ends.
tl.set({}, {}, Math.max(0, DUR - 1 / 30));
// Exit flash: a brass streak wipes the frame on the last half beat.
function exitWipe(sel) {
  tl.fromTo(sel, { xPercent: -120 }, { xPercent: 120, duration: b(0.5), ease: "power3.in" }, DUR - b(0.5));
}
// Land a block of words one by one on beats, blur to sharp, then hold still.
function wordsIn(sel, at, gap) {
  tl.fromTo(sel, { opacity: 0, filter: "blur(10px)", scale: 1.08 }, { opacity: 1, filter: "blur(0px)", scale: 1, duration: 0.16, ease: "power4.out", stagger: gap }, at);
}
