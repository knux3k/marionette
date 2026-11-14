// Moves the benchmark's movers exactly as Marionette's built-in `Follow` and `integrate`
// systems do (decision 0284): each fixed step, velocity += (anchor - position) * gain * dt,
// then position += velocity * dt. One component updates every mover, as a game would.
using UnityEngine;

namespace MarionetteBench
{
    public class RealisticBenchMovers : MonoBehaviour
    {
        public Transform[] movers;
        public Vector3[] positions;
        public Vector3[] velocities;
        public Vector3[] anchors;
        public int[] anchorOf;
        public float[] gains;
        public float dt = 1f / 60f;

        void Awake()
        {
            Time.fixedDeltaTime = dt;
        }

        void FixedUpdate()
        {
            for (int i = 0; i < movers.Length; i++)
            {
                Vector3 a = anchors[anchorOf[i]];
                Vector3 p = positions[i];
                float k = gains[i] * dt;
                Vector3 v = velocities[i] + (a - p) * k;
                p += v * dt;
                velocities[i] = v;
                positions[i] = p;
                movers[i].position = p;
            }
        }
    }
}
