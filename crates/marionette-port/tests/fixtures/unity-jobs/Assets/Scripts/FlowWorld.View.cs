using UnityEngine;

namespace Sample.Flow
{
    public partial class FlowWorld : MonoBehaviour
    {
        public float spacing = 0.25f;

        void Update()
        {
            Step();
            transform.position = new Vector3(Mathf.RoundToInt(spacing * 10), 0, 0);
        }

        void OnDestroy()
        {
            if (m_Cells.IsCreated) m_Cells.Dispose();
        }
    }
}
